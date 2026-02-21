//! Parquet writers for BM25 index data staging to S3.
//!
//! # Implements
//!
//! - **RET-01**: Parquet staging for BM25 Iceberg table population
//!
//! # WHY Parquet Staging
//!
//! AWS documentation explicitly warns against INSERT INTO...VALUES for Athena:
//! "We do not recommend inserting rows using VALUES because Athena generates
//! files for each INSERT operation. This can cause many small files to be
//! created and degrade the table's query performance."
//!
//! Instead, we:
//! 1. Write Parquet files to an S3 staging prefix
//! 2. Create a temporary external table pointing to staged files
//! 3. INSERT INTO {iceberg_table} SELECT * FROM {staging_table}
//! 4. Drop temporary table and clean up staging files
//!
//! This preserves Iceberg metadata consistency and avoids the small-file
//! anti-pattern.

use std::sync::Arc;

use arrow::array::{Int32Array, Int64Array, Float64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use tracing::debug;
use uuid::Uuid;

use crate::error::{AwsStorageError, Result};

/// Write postings data (term, doc_id, tf) as a Parquet file to S3.
///
/// # Arguments
///
/// * `s3_client` - Pre-configured S3 client
/// * `bucket` - S3 bucket name
/// * `key_prefix` - S3 key prefix for the staging location
/// * `postings` - Postings data as (term, doc_id, tf) tuples
///
/// # Returns
///
/// The S3 key of the written Parquet file.
pub async fn write_postings_parquet(
    s3_client: &aws_sdk_s3::Client,
    bucket: &str,
    key_prefix: &str,
    postings: &[(String, String, i32)],
) -> Result<String> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("term", DataType::Utf8, false),
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("tf", DataType::Int32, false),
    ]));

    let terms: StringArray = postings.iter().map(|(t, _, _)| Some(t.as_str())).collect();
    let doc_ids: StringArray = postings.iter().map(|(_, d, _)| Some(d.as_str())).collect();
    let tfs: Int32Array = postings.iter().map(|(_, _, tf)| Some(*tf)).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(terms),
            Arc::new(doc_ids),
            Arc::new(tfs),
        ],
    )
    .map_err(|e| AwsStorageError::Other(format!("Failed to create postings RecordBatch: {}", e)))?;

    write_parquet_to_s3(s3_client, bucket, key_prefix, "postings", &batch).await
}

/// Write term statistics (term, df) as a Parquet file to S3.
///
/// # Arguments
///
/// * `s3_client` - Pre-configured S3 client
/// * `bucket` - S3 bucket name
/// * `key_prefix` - S3 key prefix for the staging location
/// * `stats` - Term statistics as (term, df) tuples
///
/// # Returns
///
/// The S3 key of the written Parquet file.
pub async fn write_term_stats_parquet(
    s3_client: &aws_sdk_s3::Client,
    bucket: &str,
    key_prefix: &str,
    stats: &[(String, i32)],
) -> Result<String> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("term", DataType::Utf8, false),
        Field::new("df", DataType::Int32, false),
    ]));

    let terms: StringArray = stats.iter().map(|(t, _)| Some(t.as_str())).collect();
    let dfs: Int32Array = stats.iter().map(|(_, df)| Some(*df)).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(terms), Arc::new(dfs)],
    )
    .map_err(|e| {
        AwsStorageError::Other(format!("Failed to create term_stats RecordBatch: {}", e))
    })?;

    write_parquet_to_s3(s3_client, bucket, key_prefix, "term_stats", &batch).await
}

/// Write document metadata (doc_id, doc_length, source_id) as a Parquet file to S3.
///
/// # Arguments
///
/// * `s3_client` - Pre-configured S3 client
/// * `bucket` - S3 bucket name
/// * `key_prefix` - S3 key prefix for the staging location
/// * `docs` - Document metadata as (doc_id, doc_length, source_id) tuples
///
/// # Returns
///
/// The S3 key of the written Parquet file.
pub async fn write_docs_parquet(
    s3_client: &aws_sdk_s3::Client,
    bucket: &str,
    key_prefix: &str,
    docs: &[(String, i32, String)],
) -> Result<String> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_length", DataType::Int32, false),
        Field::new("source_id", DataType::Utf8, false),
    ]));

    let doc_ids: StringArray = docs.iter().map(|(d, _, _)| Some(d.as_str())).collect();
    let lengths: Int32Array = docs.iter().map(|(_, l, _)| Some(*l)).collect();
    let source_ids: StringArray = docs.iter().map(|(_, _, s)| Some(s.as_str())).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(doc_ids),
            Arc::new(lengths),
            Arc::new(source_ids),
        ],
    )
    .map_err(|e| AwsStorageError::Other(format!("Failed to create docs RecordBatch: {}", e)))?;

    write_parquet_to_s3(s3_client, bucket, key_prefix, "docs", &batch).await
}

/// Write corpus statistics (n, avgdl) as a Parquet file to S3.
///
/// This is typically not needed since corpus stats are computed by Athena SQL,
/// but provided for completeness.
///
/// # Arguments
///
/// * `s3_client` - Pre-configured S3 client
/// * `bucket` - S3 bucket name
/// * `key_prefix` - S3 key prefix for the staging location
/// * `n` - Total document count
/// * `avgdl` - Average document length
///
/// # Returns
///
/// The S3 key of the written Parquet file.
pub async fn write_corpus_stats_parquet(
    s3_client: &aws_sdk_s3::Client,
    bucket: &str,
    key_prefix: &str,
    n: i64,
    avgdl: f64,
) -> Result<String> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("n", DataType::Int64, false),
        Field::new("avgdl", DataType::Float64, false),
    ]));

    let ns: Int64Array = vec![Some(n)].into_iter().collect();
    let avgdls: Float64Array = vec![Some(avgdl)].into_iter().collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(ns), Arc::new(avgdls)],
    )
    .map_err(|e| {
        AwsStorageError::Other(format!(
            "Failed to create corpus_stats RecordBatch: {}",
            e
        ))
    })?;

    write_parquet_to_s3(s3_client, bucket, key_prefix, "corpus_stats", &batch).await
}

/// Internal helper: serialize a RecordBatch to Parquet bytes and upload to S3.
async fn write_parquet_to_s3(
    s3_client: &aws_sdk_s3::Client,
    bucket: &str,
    key_prefix: &str,
    file_name: &str,
    batch: &RecordBatch,
) -> Result<String> {
    // Serialize RecordBatch to Parquet bytes in memory
    let mut buf = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut buf, batch.schema(), None)
        .map_err(|e| AwsStorageError::Other(format!("Failed to create Parquet writer: {}", e)))?;
    writer.write(batch).map_err(|e| {
        AwsStorageError::Other(format!("Failed to write Parquet batch: {}", e))
    })?;
    writer.close().map_err(|e| {
        AwsStorageError::Other(format!("Failed to close Parquet writer: {}", e))
    })?;

    // Upload to S3
    let s3_key = format!("{}/{}_{}.parquet", key_prefix, file_name, Uuid::new_v4());

    debug!(
        "Uploading Parquet file to s3://{}/{} ({} bytes)",
        bucket,
        s3_key,
        buf.len()
    );

    s3_client
        .put_object()
        .bucket(bucket)
        .key(&s3_key)
        .body(aws_sdk_s3::primitives::ByteStream::from(buf))
        .send()
        .await
        .map_err(|e| AwsStorageError::S3Error(format!("Failed to upload Parquet to S3: {}", e)))?;

    Ok(s3_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_postings_schema() {
        let schema = Schema::new(vec![
            Field::new("term", DataType::Utf8, false),
            Field::new("doc_id", DataType::Utf8, false),
            Field::new("tf", DataType::Int32, false),
        ]);
        assert_eq!(schema.fields().len(), 3);
        assert_eq!(schema.field(0).name(), "term");
        assert_eq!(schema.field(1).name(), "doc_id");
        assert_eq!(schema.field(2).name(), "tf");
    }

    #[test]
    fn test_term_stats_schema() {
        let schema = Schema::new(vec![
            Field::new("term", DataType::Utf8, false),
            Field::new("df", DataType::Int32, false),
        ]);
        assert_eq!(schema.fields().len(), 2);
    }

    #[test]
    fn test_docs_schema() {
        let schema = Schema::new(vec![
            Field::new("doc_id", DataType::Utf8, false),
            Field::new("doc_length", DataType::Int32, false),
            Field::new("source_id", DataType::Utf8, false),
        ]);
        assert_eq!(schema.fields().len(), 3);
    }

    #[test]
    fn test_corpus_stats_schema() {
        let schema = Schema::new(vec![
            Field::new("n", DataType::Int64, false),
            Field::new("avgdl", DataType::Float64, false),
        ]);
        assert_eq!(schema.fields().len(), 2);
        assert_eq!(*schema.field(0).data_type(), DataType::Int64);
        assert_eq!(*schema.field(1).data_type(), DataType::Float64);
    }

    #[test]
    fn test_postings_record_batch_creation() {
        let postings = vec![
            ("hello".to_string(), "doc1".to_string(), 2i32),
            ("world".to_string(), "doc1".to_string(), 1i32),
        ];

        let schema = Arc::new(Schema::new(vec![
            Field::new("term", DataType::Utf8, false),
            Field::new("doc_id", DataType::Utf8, false),
            Field::new("tf", DataType::Int32, false),
        ]));

        let terms: StringArray = postings.iter().map(|(t, _, _)| Some(t.as_str())).collect();
        let doc_ids: StringArray = postings.iter().map(|(_, d, _)| Some(d.as_str())).collect();
        let tfs: Int32Array = postings.iter().map(|(_, _, tf)| Some(*tf)).collect();

        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(terms), Arc::new(doc_ids), Arc::new(tfs)],
        );
        assert!(batch.is_ok());
        assert_eq!(batch.unwrap().num_rows(), 2);
    }

    #[test]
    fn test_parquet_serialization() {
        // Test that we can serialize a RecordBatch to Parquet bytes
        let schema = Arc::new(Schema::new(vec![
            Field::new("term", DataType::Utf8, false),
            Field::new("df", DataType::Int32, false),
        ]));

        let terms: StringArray = vec![Some("hello"), Some("world")].into_iter().collect();
        let dfs: Int32Array = vec![Some(5), Some(3)].into_iter().collect();

        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(terms), Arc::new(dfs)]).unwrap();

        let mut buf = Vec::new();
        let mut writer = ArrowWriter::try_new(&mut buf, batch.schema(), None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        // Parquet files have a magic number
        assert!(buf.len() > 4);
        assert_eq!(&buf[0..4], b"PAR1");
    }
}
