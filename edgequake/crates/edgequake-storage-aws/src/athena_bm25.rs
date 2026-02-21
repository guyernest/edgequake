//! Amazon Athena BM25 storage implementation using Iceberg tables.
//!
//! # Implements
//!
//! - **RET-01**: BM25 inverted index via Athena Iceberg tables on S3
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────┐   write Parquet    ┌─────────┐
//! │  Batch Store │ ─────────────────> │   S3    │ (staging)
//! └──────────────┘                    └────┬────┘
//!                                          │
//!                   INSERT INTO...SELECT   │
//!                   ┌──────────────────────┘
//!                   ▼
//! ┌──────────────────┐                ┌─────────┐
//! │  Athena (Iceberg) │ ──── data ──> │   S3    │ (Iceberg tables)
//! └──────────────────┘                └─────────┘
//!         │
//!         │  BM25 scoring SQL
//!         ▼
//! ┌──────────────┐
//! │  Query Results│
//! └──────────────┘
//! ```
//!
//! # Tables (per namespace)
//!
//! - `{ns}_postings`: term -> doc_id -> tf
//! - `{ns}_term_stats`: term -> df
//! - `{ns}_docs`: doc_id -> doc_length, source_id
//! - `{ns}_corpus_stats`: N, avgdl (single row)

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, info};
use uuid::Uuid;

use edgequake_storage::{Bm25Document, Bm25SearchResult, Bm25Storage, StorageError};

use crate::athena_sql::AthenaQueryEngine;
use crate::bm25_parquet;
use crate::error::{AwsStorageError, Result};

/// Configuration for Athena BM25 storage.
#[derive(Debug, Clone)]
pub struct AthenaBm25Config {
    /// Athena/Glue database name
    pub database: String,
    /// S3 bucket for Iceberg table data
    pub s3_bucket: String,
    /// Base prefix for BM25 data (e.g., "bm25")
    pub s3_prefix: String,
    /// Athena workgroup (should be v3 for Iceberg support)
    pub workgroup: String,
    /// S3 location for Athena query results
    pub output_location: String,
}

/// BM25 storage implementation using Amazon Athena and Iceberg tables.
///
/// Stores BM25 inverted index data as Iceberg tables on S3, queried
/// via Athena SQL. Uses Parquet staging for batch data ingestion to
/// avoid the small-file anti-pattern from INSERT INTO...VALUES.
pub struct AthenaBm25Storage {
    config: AthenaBm25Config,
    namespace: String,
    athena: Arc<AthenaQueryEngine>,
    s3_client: aws_sdk_s3::Client,
}

impl AthenaBm25Storage {
    /// Create a new Athena BM25 storage instance.
    ///
    /// No async initialization -- clients are pre-created by the caller,
    /// consistent with the existing factory pattern from Phase 1.
    pub fn new(
        config: AthenaBm25Config,
        namespace: String,
        athena: Arc<AthenaQueryEngine>,
        s3_client: aws_sdk_s3::Client,
    ) -> Self {
        Self {
            config,
            namespace,
            athena,
            s3_client,
        }
    }

    /// Build the Iceberg DDL for creating a table.
    fn create_table_ddl(&self, table_suffix: &str, columns: &str) -> String {
        let db = &self.config.database;
        let ns = &self.namespace;
        let bucket = &self.config.s3_bucket;
        let prefix = &self.config.s3_prefix;

        format!(
            "CREATE TABLE IF NOT EXISTS {db}.{ns}_{table_suffix} (\
             {columns}\
             ) \
             LOCATION 's3://{bucket}/{prefix}/{ns}/{table_suffix}/' \
             TBLPROPERTIES ('table_type'='ICEBERG', 'format'='parquet')"
        )
    }

    /// Build the BM25 scoring SQL query.
    ///
    /// Uses the standard Okapi BM25 formula:
    /// score = SUM(IDF * TF-norm)
    /// where IDF = ln(1 + (N - df + 0.5) / (df + 0.5))
    /// and TF-norm = (tf * (k1 + 1)) / (tf + k1 * (1 - b + b * dl / avgdl))
    fn build_bm25_query(&self, terms: &[String], top_k: usize) -> String {
        let ns = &self.namespace;
        let db = &self.config.database;
        let k1 = 1.5;
        let b = 0.75;

        // Build VALUES clause with SQL injection prevention
        let values = terms
            .iter()
            .map(|t| format!("('{}')", t.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            "WITH query_terms(term) AS (VALUES {values}), \
             params AS (SELECT {k1} AS k1, {b} AS b), \
             cs AS (SELECT n, avgdl FROM {db}.{ns}_corpus_stats LIMIT 1), \
             matched AS (\
                 SELECT p.doc_id, p.term, p.tf, ts.df, d.doc_length \
                 FROM {db}.{ns}_postings p \
                 JOIN query_terms qt ON p.term = qt.term \
                 JOIN {db}.{ns}_term_stats ts ON p.term = ts.term \
                 JOIN {db}.{ns}_docs d ON p.doc_id = d.doc_id\
             ), \
             scored AS (\
                 SELECT m.doc_id, \
                     SUM(\
                         LN(1.0 + (cs.n - m.df + 0.5) / (m.df + 0.5)) \
                         * (m.tf * (params.k1 + 1.0)) \
                         / (m.tf + params.k1 * (1.0 - params.b + params.b * CAST(m.doc_length AS double) / cs.avgdl))\
                     ) AS bm25_score \
                 FROM matched m CROSS JOIN cs CROSS JOIN params \
                 GROUP BY m.doc_id\
             ) \
             SELECT doc_id, bm25_score FROM scored ORDER BY bm25_score DESC LIMIT {top_k}"
        )
    }

    /// Create a temporary external table pointing to staged Parquet files.
    fn create_staging_table_ddl(
        &self,
        table_name: &str,
        columns: &str,
        s3_location: &str,
    ) -> String {
        let db = &self.config.database;
        format!(
            "CREATE EXTERNAL TABLE {db}.{table_name} (\
             {columns}\
             ) \
             STORED AS PARQUET \
             LOCATION '{s3_location}'"
        )
    }

    /// Build postings, term_stats, and docs data from a batch of BM25 documents.
    fn build_index_data(
        docs: &[Bm25Document],
    ) -> (
        Vec<(String, String, i32)>,   // postings: (term, doc_id, tf)
        Vec<(String, i32)>,           // term_stats: (term, df)
        Vec<(String, i32, String)>,   // docs: (doc_id, doc_length, source_id)
    ) {
        let mut postings = Vec::new();
        let mut term_doc_count: HashMap<String, i32> = HashMap::new();
        let mut docs_meta = Vec::new();

        for doc in docs {
            // Build postings: count term frequency per document
            let mut tf_map: HashMap<&str, i32> = HashMap::new();
            for term in &doc.terms {
                *tf_map.entry(term.as_str()).or_default() += 1;
            }

            for (term, tf) in &tf_map {
                postings.push((term.to_string(), doc.doc_id.clone(), *tf));
            }

            // Track document frequency per term (number of docs containing term)
            for term in tf_map.keys() {
                *term_doc_count.entry(term.to_string()).or_default() += 1;
            }

            // Build docs metadata
            docs_meta.push((
                doc.doc_id.clone(),
                doc.doc_length as i32,
                doc.source_id.clone(),
            ));
        }

        let term_stats: Vec<(String, i32)> = term_doc_count.into_iter().collect();

        (postings, term_stats, docs_meta)
    }

    /// Delete S3 objects at a given prefix (cleanup staging files).
    async fn cleanup_staging(&self, prefix: &str) -> Result<()> {
        let response = self
            .s3_client
            .list_objects_v2()
            .bucket(&self.config.s3_bucket)
            .prefix(prefix)
            .send()
            .await
            .map_err(|e| AwsStorageError::S3Error(format!("Failed to list staging objects: {}", e)))?;

        let contents = response.contents();
        for obj in contents {
            if let Some(key) = obj.key() {
                self.s3_client
                    .delete_object()
                    .bucket(&self.config.s3_bucket)
                    .key(key)
                    .send()
                    .await
                    .map_err(|e| {
                        AwsStorageError::S3Error(format!(
                            "Failed to delete staging object {}: {}",
                            key, e
                        ))
                    })?;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Bm25Storage for AthenaBm25Storage {
    fn namespace(&self) -> &str {
        &self.namespace
    }

    async fn ensure_tables(&self) -> std::result::Result<(), StorageError> {
        let ddl_statements = vec![
            self.create_table_ddl("postings", "term string, doc_id string, tf int"),
            self.create_table_ddl("term_stats", "term string, df int"),
            self.create_table_ddl("docs", "doc_id string, doc_length int, source_id string"),
            self.create_table_ddl("corpus_stats", "n bigint, avgdl double"),
        ];

        for ddl in ddl_statements {
            debug!("Executing BM25 DDL: {}", ddl);
            self.athena
                .query(&ddl)
                .await
                .map_err(|e| StorageError::Database(format!("BM25 DDL failed: {}", e)))?;
        }

        info!(
            "BM25 Iceberg tables ensured for namespace '{}'",
            self.namespace
        );
        Ok(())
    }

    async fn index_batch(&self, docs: &[Bm25Document]) -> std::result::Result<(), StorageError> {
        if docs.is_empty() {
            return Ok(());
        }

        let batch_id = Uuid::new_v4().to_string();
        let staging_prefix = format!("bm25-staging/{}/{}", self.namespace, batch_id);
        let db = &self.config.database;
        let ns = &self.namespace;

        info!(
            "Indexing {} BM25 documents for namespace '{}' (batch {})",
            docs.len(),
            self.namespace,
            batch_id
        );

        // 1. Build in-memory index data
        let (postings, term_stats, docs_meta) = Self::build_index_data(docs);

        // 2. Write Parquet files to S3 staging
        let postings_key = bm25_parquet::write_postings_parquet(
            &self.s3_client,
            &self.config.s3_bucket,
            &format!("{}/postings", staging_prefix),
            &postings,
        )
        .await
        .map_err(|e| StorageError::Database(format!("Failed to write postings parquet: {}", e)))?;

        let term_stats_key = bm25_parquet::write_term_stats_parquet(
            &self.s3_client,
            &self.config.s3_bucket,
            &format!("{}/term_stats", staging_prefix),
            &term_stats,
        )
        .await
        .map_err(|e| {
            StorageError::Database(format!("Failed to write term_stats parquet: {}", e))
        })?;

        let docs_key = bm25_parquet::write_docs_parquet(
            &self.s3_client,
            &self.config.s3_bucket,
            &format!("{}/docs", staging_prefix),
            &docs_meta,
        )
        .await
        .map_err(|e| StorageError::Database(format!("Failed to write docs parquet: {}", e)))?;

        debug!(
            "Staged parquet files: postings={}, term_stats={}, docs={}",
            postings_key, term_stats_key, docs_key
        );

        // 3. Create temporary external tables pointing to staged Parquet
        let staging_postings_table = format!("{ns}_staging_postings_{}", batch_id.replace('-', "_"));
        let staging_term_stats_table =
            format!("{ns}_staging_term_stats_{}", batch_id.replace('-', "_"));
        let staging_docs_table = format!("{ns}_staging_docs_{}", batch_id.replace('-', "_"));

        let staging_postings_loc = format!(
            "s3://{}/{}/postings/",
            self.config.s3_bucket, staging_prefix
        );
        let staging_term_stats_loc = format!(
            "s3://{}/{}/term_stats/",
            self.config.s3_bucket, staging_prefix
        );
        let staging_docs_loc =
            format!("s3://{}/{}/docs/", self.config.s3_bucket, staging_prefix);

        // Create staging tables
        self.athena
            .query(&self.create_staging_table_ddl(
                &staging_postings_table,
                "term string, doc_id string, tf int",
                &staging_postings_loc,
            ))
            .await
            .map_err(|e| {
                StorageError::Database(format!("Failed to create staging postings table: {}", e))
            })?;

        self.athena
            .query(&self.create_staging_table_ddl(
                &staging_term_stats_table,
                "term string, df int",
                &staging_term_stats_loc,
            ))
            .await
            .map_err(|e| {
                StorageError::Database(format!(
                    "Failed to create staging term_stats table: {}",
                    e
                ))
            })?;

        self.athena
            .query(&self.create_staging_table_ddl(
                &staging_docs_table,
                "doc_id string, doc_length int, source_id string",
                &staging_docs_loc,
            ))
            .await
            .map_err(|e| {
                StorageError::Database(format!("Failed to create staging docs table: {}", e))
            })?;

        // 4. INSERT INTO...SELECT from staging tables into Iceberg tables
        self.athena
            .query(&format!(
                "INSERT INTO {db}.{ns}_postings SELECT * FROM {db}.{staging_postings_table}"
            ))
            .await
            .map_err(|e| {
                StorageError::Database(format!("Failed to insert postings: {}", e))
            })?;

        self.athena
            .query(&format!(
                "INSERT INTO {db}.{ns}_term_stats SELECT * FROM {db}.{staging_term_stats_table}"
            ))
            .await
            .map_err(|e| {
                StorageError::Database(format!("Failed to insert term_stats: {}", e))
            })?;

        self.athena
            .query(&format!(
                "INSERT INTO {db}.{ns}_docs SELECT * FROM {db}.{staging_docs_table}"
            ))
            .await
            .map_err(|e| StorageError::Database(format!("Failed to insert docs: {}", e)))?;

        // 5. Drop temporary staging tables
        let _ = self
            .athena
            .query(&format!("DROP TABLE IF EXISTS {db}.{staging_postings_table}"))
            .await;
        let _ = self
            .athena
            .query(&format!(
                "DROP TABLE IF EXISTS {db}.{staging_term_stats_table}"
            ))
            .await;
        let _ = self
            .athena
            .query(&format!("DROP TABLE IF EXISTS {db}.{staging_docs_table}"))
            .await;

        // 6. Delete staging Parquet files from S3
        if let Err(e) = self.cleanup_staging(&staging_prefix).await {
            // Non-fatal -- staging files will be cleaned up by S3 lifecycle
            tracing::warn!("Failed to cleanup staging files: {}", e);
        }

        info!(
            "Successfully indexed {} documents for namespace '{}'",
            docs.len(),
            self.namespace
        );
        Ok(())
    }

    async fn update_corpus_stats(&self) -> std::result::Result<(), StorageError> {
        let db = &self.config.database;
        let ns = &self.namespace;

        info!("Recalculating corpus stats for namespace '{}'", ns);

        // Delete existing corpus stats
        self.athena
            .query(&format!("DELETE FROM {db}.{ns}_corpus_stats"))
            .await
            .map_err(|e| {
                StorageError::Database(format!("Failed to delete corpus stats: {}", e))
            })?;

        // Insert fresh corpus stats computed from docs table
        self.athena
            .query(&format!(
                "INSERT INTO {db}.{ns}_corpus_stats \
                 SELECT COUNT(*) AS n, AVG(CAST(doc_length AS double)) AS avgdl \
                 FROM {db}.{ns}_docs"
            ))
            .await
            .map_err(|e| {
                StorageError::Database(format!("Failed to insert corpus stats: {}", e))
            })?;

        info!("Corpus stats updated for namespace '{}'", ns);
        Ok(())
    }

    async fn query(
        &self,
        terms: &[String],
        top_k: usize,
    ) -> std::result::Result<Vec<Bm25SearchResult>, StorageError> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }

        let sql = self.build_bm25_query(terms, top_k);
        debug!("BM25 query SQL: {}", sql);

        let rows = self
            .athena
            .query(&sql)
            .await
            .map_err(|e| StorageError::Database(format!("BM25 query failed: {}", e)))?;

        let results: std::result::Result<Vec<Bm25SearchResult>, StorageError> = rows
            .iter()
            .map(|row| {
                let doc_id = row
                    .get("doc_id")
                    .map_err(|e| StorageError::Database(format!("Missing doc_id: {}", e)))?;
                let score = row
                    .get_f64("bm25_score")
                    .map_err(|e| StorageError::Database(format!("Missing bm25_score: {}", e)))?;

                Ok(Bm25SearchResult {
                    doc_id,
                    score,
                    term_matches: vec![], // Could be enriched with additional query
                })
            })
            .collect();

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_athena_bm25_config() {
        let config = AthenaBm25Config {
            database: "test_db".to_string(),
            s3_bucket: "test-bucket".to_string(),
            s3_prefix: "bm25".to_string(),
            workgroup: "primary".to_string(),
            output_location: "s3://test-bucket/athena-results/".to_string(),
        };
        assert_eq!(config.database, "test_db");
        assert_eq!(config.s3_bucket, "test-bucket");
    }

    #[test]
    fn test_athena_bm25_config_fields() {
        let config = AthenaBm25Config {
            database: "mydb".to_string(),
            s3_bucket: "mybucket".to_string(),
            s3_prefix: "bm25".to_string(),
            workgroup: "primary".to_string(),
            output_location: "s3://mybucket/results/".to_string(),
        };

        assert_eq!(config.database, "mydb");
        assert_eq!(config.s3_bucket, "mybucket");
        assert_eq!(config.s3_prefix, "bm25");
        assert_eq!(config.workgroup, "primary");
        assert_eq!(config.output_location, "s3://mybucket/results/");
    }

    #[test]
    fn test_build_index_data() {
        let docs = vec![
            Bm25Document {
                doc_id: "doc1".to_string(),
                terms: vec!["hello".to_string(), "world".to_string(), "hello".to_string()],
                doc_length: 3,
                source_id: "src1".to_string(),
            },
            Bm25Document {
                doc_id: "doc2".to_string(),
                terms: vec!["hello".to_string(), "rust".to_string()],
                doc_length: 2,
                source_id: "src2".to_string(),
            },
        ];

        let (postings, term_stats, docs_meta) = AthenaBm25Storage::build_index_data(&docs);

        // Postings should have entries for each unique (term, doc_id) pair
        assert!(postings.len() >= 4); // hello/doc1, world/doc1, hello/doc2, rust/doc2

        // Term stats should track document frequency
        let hello_df = term_stats
            .iter()
            .find(|(t, _)| t == "hello")
            .map(|(_, df)| *df);
        assert_eq!(hello_df, Some(2)); // "hello" appears in 2 docs

        let world_df = term_stats
            .iter()
            .find(|(t, _)| t == "world")
            .map(|(_, df)| *df);
        assert_eq!(world_df, Some(1)); // "world" appears in 1 doc

        // Docs metadata
        assert_eq!(docs_meta.len(), 2);
        assert_eq!(docs_meta[0].0, "doc1");
        assert_eq!(docs_meta[0].1, 3);

        // Check tf for hello in doc1 is 2
        let hello_doc1_tf = postings
            .iter()
            .find(|(t, d, _)| t == "hello" && d == "doc1")
            .map(|(_, _, tf)| *tf);
        assert_eq!(hello_doc1_tf, Some(2));
    }

    #[test]
    fn test_build_bm25_query_sql_injection_prevention() {
        // Verify that single quotes in terms are escaped
        // (testing the escaping logic used in build_bm25_query)
        let term = "test'injection";
        let escaped = term.replace('\'', "''");
        assert_eq!(escaped, "test''injection");

        // Multiple quotes
        let term2 = "it's a 'test'";
        let escaped2 = term2.replace('\'', "''");
        assert_eq!(escaped2, "it''s a ''test''");
    }
}
