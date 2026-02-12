//! Pipeline stages for batch ingestion.
//!
//! Each stage corresponds to one phase of the batch pipeline:
//! - `prepare`: Phase 1 - Parse parquet, chunk documents, build JSONL
//! - `extract`: Phase 2 - Submit batch jobs, poll, download results
//! - `embed`:   Phase 3 - Generate embeddings via standard API
//! - `store`:   Phase 4 - Write to Neptune, S3, and DynamoDB

pub mod prepare;
pub mod extract;
pub mod embed;
pub mod store;
pub mod bulk_load;
