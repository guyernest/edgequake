//! DynamoDB-local-gated integration tests for [`DynamoWorkspaceService`].
//!
//! These tests are the pre-deploy behavioral backstop for the Phase 33 Wave 8
//! workspace store — they exercise the three live happy-path key shapes
//! (tenant list, get_workspace-by-id, get_workspace_by_slug) against a real
//! (local) DynamoDB before any AWS round-trip.
//!
//! They are SKIPPED in the default `cargo test` (no DynamoDB) via the
//! `DYNAMODB_LOCAL_ENDPOINT`-unset guard, so they never trip the ~26 pre-existing
//! app-state `block_on` failures. They construct ONLY a `DynamoWorkspaceService`
//! against a local DynamoDB client — never the API crate's shared application
//! state harness — so they stay hermetic.
//!
//! ## Running against DynamoDB Local
//!
//! ```bash
//! # Start DynamoDB local (e.g. amazon/dynamodb-local on :8000):
//! docker run -p 8000:8000 amazon/dynamodb-local
//!
//! # Run the round-trip:
//! DYNAMODB_LOCAL_ENDPOINT=http://localhost:8000 \
//!   cargo test -p edgequake-storage-aws --features dynamodb \
//!   --test dynamodb_workspace_test -- --nocapture ws_dynamo_roundtrip
//! ```

#![cfg(feature = "dynamodb")]

use aws_sdk_dynamodb::config::Credentials;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType, TableStatus,
};
use aws_sdk_dynamodb::Client;
use edgequake_core::types::{CreateWorkspaceRequest, Tenant};
use edgequake_core::WorkspaceService;
use edgequake_storage_aws::{DynamoWorkspaceConfig, DynamoWorkspaceService};
use uuid::Uuid;

/// Whether the DynamoDB-local integration tests should run.
///
/// Keyed on `DYNAMODB_LOCAL_ENDPOINT` (e.g. `http://localhost:8000`). When unset,
/// every test early-returns with a skip-print so default `cargo test` stays green.
fn should_run() -> Option<String> {
    std::env::var("DYNAMODB_LOCAL_ENDPOINT").ok()
}

/// The single PK/SK test table all round-trips share.
const TEST_TABLE: &str = "edgequake-test-workspaces";

/// Build a DynamoDB client pointed at the local endpoint with dummy test creds.
///
/// Hermetic: constructs only an `aws_sdk_dynamodb::Client`.
async fn local_client(endpoint: &str) -> Client {
    let creds = Credentials::new("test", "test", None, None, "dynamodb-local-test");
    let conf = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region("us-east-1")
        .endpoint_url(endpoint)
        .credentials_provider(creds)
        .load()
        .await;
    Client::new(&conf)
}

/// Ensure the single PK/SK test table exists and is ACTIVE before use.
///
/// Review edit H: tolerates a parallel-create race (treats `ResourceInUseException`
/// from CreateTable as "already exists, fall through") and waits for the table to
/// reach `TableStatus::Active` via a bounded DescribeTable poll, so concurrent test
/// binaries do not flake.
async fn ensure_table_active(client: &Client) {
    // Attempt CreateTable; tolerate ResourceInUseException (already created by a
    // sibling test binary / previous run).
    let create = client
        .create_table()
        .table_name(TEST_TABLE)
        .billing_mode(BillingMode::PayPerRequest)
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("PK")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .unwrap(),
        )
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("SK")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .unwrap(),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("PK")
                .key_type(KeyType::Hash)
                .build()
                .unwrap(),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("SK")
                .key_type(KeyType::Range)
                .build()
                .unwrap(),
        )
        .send()
        .await;

    if let Err(e) = create {
        let msg = format!("{:?}", e);
        // ResourceInUseException => table already exists; fall through to the
        // DescribeTable-until-ACTIVE wait. Any other error is fatal.
        if !msg.contains("ResourceInUse") {
            panic!("CreateTable failed (and not ResourceInUse): {}", msg);
        }
    }

    // DescribeTable poll until ACTIVE (bounded retries with a short sleep).
    for attempt in 0..30 {
        let desc = client
            .describe_table()
            .table_name(TEST_TABLE)
            .send()
            .await
            .expect("DescribeTable failed");
        let status = desc
            .table
            .and_then(|t| t.table_status)
            .unwrap_or(TableStatus::Creating);
        if status == TableStatus::Active {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if attempt == 29 {
            panic!("Test table never reached ACTIVE (last status: {:?})", status);
        }
    }
}

/// Build a Tenant fixture with deterministic-per-run unique slug/id.
fn fresh_tenant(run: &str) -> Tenant {
    Tenant::new(format!("Test Tenant {}", run), format!("test-{}", run))
        .with_embedding_config("text-embedding-3-small", "openai", 1536)
        .with_llm_config("gpt-4o-mini", "openai")
}

/// Round-trip (1): `create_tenant` then `list_tenants` returns the created tenant.
///
/// Also exercises the consistent_read path — the read must see the just-written
/// tenant immediately.
#[tokio::test]
async fn ws_dynamo_roundtrip_tenant_list() {
    let Some(endpoint) = should_run() else {
        println!("Skipping — set DYNAMODB_LOCAL_ENDPOINT to run");
        return;
    };
    let client = local_client(&endpoint).await;
    ensure_table_active(&client).await;
    let service = DynamoWorkspaceService::new(
        DynamoWorkspaceConfig {
            table_name: TEST_TABLE.to_string(),
        },
        client,
    );

    let run = Uuid::new_v4().simple().to_string();
    let tenant = fresh_tenant(&run);
    let tenant_id = tenant.tenant_id;
    service.create_tenant(tenant).await.expect("create_tenant");

    let tenants = service.list_tenants(1000, 0).await.expect("list_tenants");
    assert!(
        tenants.iter().any(|t| t.tenant_id == tenant_id),
        "list_tenants must return the just-created tenant (consistent read)"
    );
}

/// Round-trip (2): `create_workspace` then `get_workspace(workspace_id)` returns
/// it in a single GetItem (no tenant arg — Pitfall 5).
#[tokio::test]
async fn ws_dynamo_roundtrip_get_workspace_by_id() {
    let Some(endpoint) = should_run() else {
        println!("Skipping — set DYNAMODB_LOCAL_ENDPOINT to run");
        return;
    };
    let client = local_client(&endpoint).await;
    ensure_table_active(&client).await;
    let service = DynamoWorkspaceService::new(
        DynamoWorkspaceConfig {
            table_name: TEST_TABLE.to_string(),
        },
        client,
    );

    let run = Uuid::new_v4().simple().to_string();
    let tenant = fresh_tenant(&run);
    let tenant_id = tenant.tenant_id;
    service.create_tenant(tenant).await.expect("create_tenant");

    let request = CreateWorkspaceRequest {
        name: format!("KB {}", run),
        slug: Some(format!("kb-{}", run)),
        description: Some("Round-trip KB".to_string()),
        max_documents: Some(1000),
        ..Default::default()
    };
    let created = service
        .create_workspace(tenant_id, request)
        .await
        .expect("create_workspace");

    let fetched = service
        .get_workspace(created.workspace_id)
        .await
        .expect("get_workspace")
        .expect("workspace must exist after create");
    assert_eq!(fetched.workspace_id, created.workspace_id);
    assert_eq!(fetched.tenant_id, tenant_id);
    assert_eq!(fetched.slug, created.slug);
}

/// Round-trip (3): `get_workspace_by_slug(tenant_id, slug)` returns the workspace
/// via the two-hop WSBYSLUG -> WS#id lookup.
#[tokio::test]
async fn ws_dynamo_roundtrip_get_workspace_by_slug() {
    let Some(endpoint) = should_run() else {
        println!("Skipping — set DYNAMODB_LOCAL_ENDPOINT to run");
        return;
    };
    let client = local_client(&endpoint).await;
    ensure_table_active(&client).await;
    let service = DynamoWorkspaceService::new(
        DynamoWorkspaceConfig {
            table_name: TEST_TABLE.to_string(),
        },
        client,
    );

    let run = Uuid::new_v4().simple().to_string();
    let tenant = fresh_tenant(&run);
    let tenant_id = tenant.tenant_id;
    service.create_tenant(tenant).await.expect("create_tenant");

    let slug = format!("kb-slug-{}", run);
    let request = CreateWorkspaceRequest {
        name: format!("KB Slug {}", run),
        slug: Some(slug.clone()),
        ..Default::default()
    };
    let created = service
        .create_workspace(tenant_id, request)
        .await
        .expect("create_workspace");

    let by_slug = service
        .get_workspace_by_slug(tenant_id, &slug)
        .await
        .expect("get_workspace_by_slug")
        .expect("workspace must be found by slug");
    assert_eq!(by_slug.workspace_id, created.workspace_id);
    assert_eq!(by_slug.slug, slug);

    // Duplicate-slug create must fail cleanly (conditional attribute_not_exists).
    let dup = CreateWorkspaceRequest {
        name: format!("Dup {}", run),
        slug: Some(slug.clone()),
        ..Default::default()
    };
    let dup_result = service.create_workspace(tenant_id, dup).await;
    assert!(
        dup_result.is_err(),
        "duplicate slug create must fail (conditional put), not overwrite"
    );
}
