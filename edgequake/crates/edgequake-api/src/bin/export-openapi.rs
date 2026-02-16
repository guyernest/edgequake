//! Export OpenAPI schema to JSON file.
//!
//! This utility exports the EdgeQuake OpenAPI 3.0 schema to a JSON file
//! for use with MCP server generation tools.
//!
//! # Usage
//!
//! ```bash
//! cargo run --bin export-openapi > openapi.json
//! cargo run --bin export-openapi --pretty > openapi.json
//! cargo run --bin export-openapi --output openapi.json
//! ```

use edgequake_api::openapi::ApiDoc;
use std::env;
use std::fs;
use utoipa::OpenApi;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Generate OpenAPI spec
    let openapi = ApiDoc::openapi();

    // Check for formatting option
    let pretty = args.contains(&"--pretty".to_string());
    let json = if pretty {
        serde_json::to_string_pretty(&openapi).expect("Failed to serialize OpenAPI spec")
    } else {
        serde_json::to_string(&openapi).expect("Failed to serialize OpenAPI spec")
    };

    // Check for output file
    if let Some(output_idx) = args.iter().position(|arg| arg == "--output") {
        if let Some(output_path) = args.get(output_idx + 1) {
            fs::write(output_path, &json).expect("Failed to write OpenAPI spec to file");
            eprintln!("✅ OpenAPI schema exported to: {}", output_path);
            eprintln!("📊 Total endpoints: {}", count_endpoints(&openapi));
            eprintln!("📦 Total schemas: {}", count_schemas(&openapi));
        } else {
            eprintln!("❌ --output requires a file path");
            std::process::exit(1);
        }
    } else {
        // Print to stdout
        println!("{}", json);
    }
}

fn count_endpoints(openapi: &utoipa::openapi::OpenApi) -> usize {
    openapi.paths.paths.len()
}

fn count_schemas(openapi: &utoipa::openapi::OpenApi) -> usize {
    openapi
        .components
        .as_ref()
        .map(|c| c.schemas.len())
        .unwrap_or(0)
}
