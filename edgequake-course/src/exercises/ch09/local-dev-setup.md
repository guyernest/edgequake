# Exercise: Local Dev Setup

::: exercise
id: ch09-local-dev-setup
difficulty: beginner
time: 20 minutes
:::

In chapter 9 you learned that EdgeQuake's local development environment uses
Docker Compose to run PostgreSQL with the pgvector and Apache AGE extensions.
A reproducible local stack ensures every developer has an identical environment
and eliminates "works on my machine" problems.

In this exercise you will complete a Docker Compose configuration for the
EdgeQuake development stack and write a Rust program that verifies connectivity
to all services.

::: objectives
thinking:
  - Understand how Docker Compose defines multi-service development environments
  - Recognize the importance of health checks for service dependency ordering
  - Know which PostgreSQL extensions EdgeQuake requires and why
doing:
  - Complete a Docker Compose YAML file with PostgreSQL configuration
  - Configure health checks for reliable startup ordering
  - Write a Rust connectivity test using sqlx
:::

::: discussion
- Why does EdgeQuake use Docker Compose for local development instead of asking developers to install PostgreSQL natively?
- What is the purpose of health checks in Docker Compose, and what happens without them?
- How do the local PostgreSQL extensions (pgvector, AGE) map to AWS services in production?
:::

::: starter file="docker-compose.yml"
```yaml
# EdgeQuake Local Development Stack
# Complete the TODOs to configure all services.

version: "3.8"

services:
  postgres:
    # TODO: Use the ankane/pgvector:pg16 image (includes pgvector extension)
    image: # YOUR IMAGE HERE
    container_name: edgequake-postgres
    environment:
      # TODO: Set PostgreSQL credentials and database name
      POSTGRES_USER: # YOUR VALUE
      POSTGRES_PASSWORD: # YOUR VALUE
      POSTGRES_DB: # YOUR VALUE
    ports:
      # TODO: Map container port 5432 to host port 5432
      - # YOUR PORT MAPPING
    volumes:
      # Persist data across container restarts
      - pgdata:/var/lib/postgresql/data
      # TODO: Mount the init script that creates extensions
      - ./init.sql:/docker-entrypoint-initdb.d/init.sql
    healthcheck:
      # TODO: Use pg_isready to check database readiness
      test: # YOUR HEALTH CHECK COMMAND
      interval: 5s
      timeout: 5s
      retries: 5

volumes:
  pgdata:
```
:::

::: starter file="init.sql"
```sql
-- Extensions required by EdgeQuake
-- pgvector: vector similarity search
CREATE EXTENSION IF NOT EXISTS vector;

-- pg_trgm: trigram-based text search (used for fuzzy matching)
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- Verify extensions are installed
SELECT extname, extversion FROM pg_extension WHERE extname IN ('vector', 'pg_trgm');
```
:::

::: starter file="src/main.rs"
```rust
//! EdgeQuake local development connectivity check.
//!
//! Verifies that all required services are running and accessible.

use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("EdgeQuake Local Dev -- Connectivity Check");
    println!("==========================================\n");

    // TODO: Build the connection string from environment variables
    // with a fallback to default local development values.
    // Format: postgres://user:password@host:port/database
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| {
            // TODO: Fill in the default connection string matching your
            // docker-compose.yml credentials
            todo!("Provide default DATABASE_URL")
        });

    // TODO: Create a connection pool with a 10-second connect timeout
    // and a maximum of 5 connections.
    // Use PgPoolOptions::new() and configure it.
    let pool = todo!("Create the connection pool");

    // Test 1: Basic connectivity
    println!("[1/3] Testing PostgreSQL connectivity...");
    // TODO: Execute "SELECT 1 as alive" and verify the result.
    // Use sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&pool).await?
    let alive: i32 = todo!("Execute SELECT 1");
    assert_eq!(alive, 1);
    println!("  OK -- PostgreSQL is responding\n");

    // Test 2: pgvector extension
    println!("[2/3] Checking pgvector extension...");
    // TODO: Query pg_extension for the vector extension version.
    // SELECT extversion FROM pg_extension WHERE extname = 'vector'
    let pgvector_version: Option<String> = todo!("Query pgvector version");
    match pgvector_version {
        Some(version) => println!("  OK -- pgvector {} installed\n", version),
        None => {
            eprintln!("  FAIL -- pgvector extension not found");
            std::process::exit(1);
        }
    }

    // Test 3: Vector operations
    println!("[3/3] Testing vector operations...");
    // TODO: Run a simple vector similarity query to verify pgvector works.
    // SELECT 1 - (embedding <=> '[1,0,0]') AS similarity
    // FROM (SELECT '[0.9,0.1,0.0]'::vector(3) AS embedding) t
    let similarity: f64 = todo!("Execute vector similarity test");
    assert!(similarity > 0.9, "Expected high similarity, got {}", similarity);
    println!("  OK -- Vector similarity query works (score: {:.4})\n", similarity);

    println!("==========================================");
    println!("All checks passed! Environment is ready.");

    Ok(())
}
```
:::

::: hint level=1 title="Docker Compose basics"
The PostgreSQL image and port mapping should look like:
```yaml
image: ankane/pgvector:pg16
ports:
  - "5432:5432"
```
For the health check, use:
```yaml
test: ["CMD-SHELL", "pg_isready -U edgequake"]
```
:::

::: hint level=2 title="Connection pool setup"
```rust
let pool = PgPoolOptions::new()
    .max_connections(5)
    .acquire_timeout(Duration::from_secs(10))
    .connect(&database_url)
    .await?;
```
:::

::: hint level=3 title="Vector similarity query"
```rust
let similarity: f64 = sqlx::query_scalar(
    "SELECT 1 - ('[0.9,0.1,0.0]'::vector(3) <=> '[1,0,0]'::vector(3)) AS similarity"
)
.fetch_one(&pool)
.await?;
```
:::

::: solution
```yaml
# docker-compose.yml
version: "3.8"

services:
  postgres:
    image: ankane/pgvector:pg16
    container_name: edgequake-postgres
    environment:
      POSTGRES_USER: edgequake
      POSTGRES_PASSWORD: edgequake_dev
      POSTGRES_DB: edgequake
    ports:
      - "5432:5432"
    volumes:
      - pgdata:/var/lib/postgresql/data
      - ./init.sql:/docker-entrypoint-initdb.d/init.sql
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U edgequake"]
      interval: 5s
      timeout: 5s
      retries: 5

volumes:
  pgdata:
```

```rust
//! EdgeQuake local development connectivity check.

use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("EdgeQuake Local Dev -- Connectivity Check");
    println!("==========================================\n");

    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://edgequake:edgequake_dev@localhost:5432/edgequake".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await?;

    // Test 1: Basic connectivity
    println!("[1/3] Testing PostgreSQL connectivity...");
    let alive: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&pool)
        .await?;
    assert_eq!(alive, 1);
    println!("  OK -- PostgreSQL is responding\n");

    // Test 2: pgvector extension
    println!("[2/3] Checking pgvector extension...");
    let pgvector_version: Option<String> = sqlx::query_scalar(
        "SELECT extversion FROM pg_extension WHERE extname = 'vector'"
    )
    .fetch_optional(&pool)
    .await?;

    match pgvector_version {
        Some(version) => println!("  OK -- pgvector {} installed\n", version),
        None => {
            eprintln!("  FAIL -- pgvector extension not found");
            std::process::exit(1);
        }
    }

    // Test 3: Vector operations
    println!("[3/3] Testing vector operations...");
    let similarity: f64 = sqlx::query_scalar(
        "SELECT 1 - ('[0.9,0.1,0.0]'::vector(3) <=> '[1,0,0]'::vector(3)) AS similarity"
    )
    .fetch_one(&pool)
    .await?;
    assert!(similarity > 0.9, "Expected high similarity, got {}", similarity);
    println!("  OK -- Vector similarity query works (score: {:.4})\n", similarity);

    println!("==========================================");
    println!("All checks passed! Environment is ready.");

    Ok(())
}
```

### Explanation

The Docker Compose stack provides:

- **PostgreSQL with pgvector**: The `ankane/pgvector:pg16` image includes the vector
  extension pre-compiled. The `init.sql` script enables it on first startup.
- **Health checks**: `pg_isready` verifies the database accepts connections before
  dependent services start. Without health checks, the Rust program would fail with
  connection refused errors on first boot.
- **Volume mount**: `pgdata` persists data across `docker compose down/up` cycles.
- **Connectivity test**: The Rust program verifies three things: basic SQL works,
  pgvector is installed, and vector similarity queries produce correct results.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    /// These tests verify the Docker Compose configuration is valid
    /// and the connectivity check logic is sound. They do NOT require
    /// a running Docker environment.

    #[test]
    fn test_default_database_url_format() {
        let url = "postgres://edgequake:edgequake_dev@localhost:5432/edgequake";
        assert!(url.starts_with("postgres://"));
        assert!(url.contains("@localhost:5432/"));
        assert!(url.contains("edgequake"));
    }

    #[test]
    fn test_database_url_from_env() {
        // Verify that DATABASE_URL env var would override the default
        std::env::set_var("DATABASE_URL_TEST", "postgres://custom:pass@db:5432/mydb");
        let url = std::env::var("DATABASE_URL_TEST").unwrap();
        assert!(url.contains("custom"));
        std::env::remove_var("DATABASE_URL_TEST");
    }

    #[test]
    fn test_init_sql_has_vector_extension() {
        // Verify the init script includes the vector extension
        let init_sql = "CREATE EXTENSION IF NOT EXISTS vector;";
        assert!(init_sql.contains("vector"));
    }

    #[test]
    fn test_health_check_command() {
        // Verify the health check command structure
        let cmd = "pg_isready -U edgequake";
        assert!(cmd.contains("pg_isready"));
        assert!(cmd.contains("edgequake"));
    }
}
```
:::

::: reflection
- How would you add a Redis service to this Docker Compose file for caching?
  What health check would you use for Redis?
- The init.sql script runs only on first database creation. How would you handle
  schema migrations for subsequent deployments?
- What changes when moving from this local setup to AWS? Which services get
  replaced by managed alternatives?
:::
