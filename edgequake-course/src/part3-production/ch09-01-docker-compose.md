## 9.1 Docker Compose Stack

EdgeQuake's local development environment is defined in a single
`docker-compose.yml` file that brings up PostgreSQL (with extensions), the
Rust API server, and optionally a Next.js frontend. Let us walk through each
service and understand the design decisions.

### The Full Docker Compose File

```yaml
services:
  # EdgeQuake API Server
  edgequake:
    build:
      context: ..
      dockerfile: docker/Dockerfile
    container_name: edgequake
    restart: unless-stopped
    ports:
      - "${EDGEQUAKE_PORT:-8080}:8080"
    environment:
      - EDGEQUAKE_HOST=0.0.0.0
      - EDGEQUAKE_PORT=8080
      - DATABASE_URL=postgres://edgequake:${POSTGRES_PASSWORD:-edgequake_secret}@postgres:5432/edgequake
      - OPENAI_API_KEY=${OPENAI_API_KEY:-}
      - RUST_LOG=info,edgequake=debug
    depends_on:
      postgres:
        condition: service_healthy
    networks:
      - edgequake-network
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 10s

  # PostgreSQL with pgvector and Apache AGE extensions
  postgres:
    build:
      context: .
      dockerfile: Dockerfile.postgres
    container_name: edgequake-postgres
    restart: unless-stopped
    ports:
      - "${POSTGRES_PORT:-5432}:5432"
    environment:
      - POSTGRES_USER=edgequake
      - POSTGRES_PASSWORD=${POSTGRES_PASSWORD:-edgequake_secret}
      - POSTGRES_DB=edgequake
    volumes:
      - postgres-data:/var/lib/postgresql/data
      - ./init-extensions.sql:/docker-entrypoint-initdb.d/init.sql:ro
    networks:
      - edgequake-network
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U edgequake -d edgequake"]
      interval: 10s
      timeout: 5s
      retries: 5
      start_period: 10s

volumes:
  postgres-data:
    driver: local

networks:
  edgequake-network:
    driver: bridge
```

### Service-by-Service Walkthrough

#### PostgreSQL Service

The `postgres` service is the foundation. It uses a custom Dockerfile
(`Dockerfile.postgres`) that builds pgvector and Apache AGE from source on
top of the official PostgreSQL 16 image:

```dockerfile
FROM postgres:16-bookworm

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    postgresql-server-dev-16 \
    curl git flex bison \
    && rm -rf /var/lib/apt/lists/*

# Build and install pgvector
RUN cd /tmp && \
    git clone --branch v0.7.4 https://github.com/pgvector/pgvector.git && \
    cd pgvector && make && make install && rm -rf /tmp/pgvector

# Build and install Apache AGE
RUN cd /tmp && \
    git clone --branch PG16/v1.6.0-rc0 https://github.com/apache/age.git && \
    cd age && make && make install && rm -rf /tmp/age

# Clean up build dependencies
RUN apt-get remove -y build-essential postgresql-server-dev-16 git \
    && apt-get autoremove -y && rm -rf /var/lib/apt/lists/*
```

Building from source ensures we get exactly the versions we need. The cleanup
step removes build tools to reduce the final image size.

#### Volume Mounts

Two volumes are mounted into the PostgreSQL container:

| Mount | Purpose |
|-------|---------|
| `postgres-data:/var/lib/postgresql/data` | Persists database data across container restarts |
| `./init-extensions.sql:/docker-entrypoint-initdb.d/init.sql:ro` | Runs extension initialization on first start |

The `init-extensions.sql` file is mounted read-only (`:ro`) into PostgreSQL's
`docker-entrypoint-initdb.d` directory. Files in this directory are executed
automatically when the database is first created (but not on subsequent starts).

```sql
-- init-extensions.sql
ALTER USER edgequake SET search_path TO public;

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "vector";

-- Apache AGE is optional -- if it fails to load, graph features
-- use the fallback SQL-based storage.
DO $$
BEGIN
    CREATE EXTENSION IF NOT EXISTS "age" CASCADE;
    RAISE NOTICE 'Apache AGE extension enabled successfully';
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'Apache AGE not available: %. Using fallback.', SQLERRM;
END $$;
```

#### Health Checks

Health checks are essential for service dependency ordering. Without them,
`depends_on` only waits for the container to start, not for the service inside
to be ready.

```yaml
# PostgreSQL: checks if the database accepts connections
healthcheck:
  test: ["CMD-SHELL", "pg_isready -U edgequake -d edgequake"]
  interval: 10s
  timeout: 5s
  retries: 5
  start_period: 10s

# EdgeQuake API: checks the /health endpoint
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
  interval: 30s
  timeout: 10s
  retries: 3
  start_period: 10s
```

The `start_period` gives the container time to initialize before health checks
begin. PostgreSQL with AGE compilation can take 10-15 seconds on first start.

#### Dependency Ordering

```yaml
edgequake:
  depends_on:
    postgres:
      condition: service_healthy
```

The `condition: service_healthy` ensures the API server does not start until
PostgreSQL passes its health check. This prevents connection errors during
startup.

```mermaid
sequenceDiagram
    participant DC as Docker Compose
    participant PG as PostgreSQL
    participant EQ as EdgeQuake API

    DC->>PG: Start container
    PG->>PG: Initialize data directory
    PG->>PG: Run init-extensions.sql
    PG->>PG: Accept connections
    DC->>PG: Health check: pg_isready
    PG-->>DC: Ready
    DC->>EQ: Start container
    EQ->>PG: Connect (DATABASE_URL)
    EQ->>EQ: Run SQLx migrations
    EQ->>EQ: Listen on :8080
```

#### Networking

All services join the `edgequake-network` bridge network. Within this network,
containers reference each other by service name:

```
DATABASE_URL=postgres://edgequake:secret@postgres:5432/edgequake
                                         ^^^^^^^
                                     Docker service name, not "localhost"
```

From the host machine, services are accessible via the mapped ports:

| Service | Container Port | Host Port |
|---------|---------------|-----------|
| PostgreSQL | 5432 | `${POSTGRES_PORT:-5432}` |
| EdgeQuake API | 8080 | `${EDGEQUAKE_PORT:-8080}` |
| Frontend | 3000 | `${FRONTEND_PORT:-3000}` |

### Common Operations

#### Starting the Stack

```bash
# Start all services in the background
docker compose up -d

# Start only PostgreSQL (for running EdgeQuake on the host)
docker compose up -d postgres

# Watch logs
docker compose logs -f edgequake
```

#### Connecting Directly to PostgreSQL

```bash
# Via Docker
docker exec -it edgequake-postgres psql -U edgequake -d edgequake

# From host (if port 5432 is mapped)
psql -h localhost -U edgequake -d edgequake
```

#### Resetting the Database

```bash
# Stop containers and remove the data volume
docker compose down -v

# Restart -- init-extensions.sql runs again on fresh volume
docker compose up -d
```

#### Running EdgeQuake on the Host (Not in Docker)

During active development, you often want to run EdgeQuake directly on the host
for faster edit-compile-run cycles while keeping PostgreSQL in Docker:

```bash
# Start only the database
docker compose up -d postgres

# Run EdgeQuake on the host
export DATABASE_URL="postgres://edgequake:edgequake_secret@localhost:5432/edgequake"
export OPENAI_API_KEY="sk-..."
cargo run --release -p edgequake-api
```

### Makefile Integration

Following EdgeQuake's convention of using Makefiles for all project scripts:

```makefile
# In the project Makefile
.PHONY: dev dev-db dev-down dev-reset

dev:  ## Start full local stack
	docker compose -f docker/docker-compose.yml up -d

dev-db:  ## Start only PostgreSQL
	docker compose -f docker/docker-compose.yml up -d postgres

dev-down:  ## Stop all services
	docker compose -f docker/docker-compose.yml down

dev-reset:  ## Stop services and destroy data
	docker compose -f docker/docker-compose.yml down -v
```

### Summary

The Docker Compose stack provides a reproducible local environment with a
custom PostgreSQL image containing pgvector and Apache AGE. Health checks
ensure proper service ordering, named volumes persist data across restarts,
and an initialization script creates extensions on first run. For active
development, you can run just the database in Docker while running EdgeQuake
natively on the host.

---

*Next: [9.2 PostgreSQL with pgvector and AGE](ch09-02-pgvector-age.md)*
