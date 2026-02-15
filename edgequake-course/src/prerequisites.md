# Prerequisites

Before starting this course, ensure you have the following background knowledge and tools installed.

## Knowledge Requirements

### Rust Basics
You should be comfortable with:
- Ownership and borrowing
- Traits and trait objects (`dyn Trait`, `impl Trait`)
- Async/await with `tokio`
- Error handling (`Result`, `?` operator)
- Cargo workspaces and dependencies

If you need a refresher, the [Rust Book](https://doc.rust-lang.org/book/) covers these topics thoroughly.

### RAG Concepts
You should understand at a high level:
- What embeddings are (vector representations of text)
- What vector similarity search does (finding semantically similar content)
- The basic RAG pattern (retrieve relevant context, then generate with an LLM)

Part I provides a thorough review, but prior exposure helps.

### Docker
Several chapters use Docker for local development:
- Running `docker compose up`
- Understanding container networking
- Basic Dockerfile concepts

## Tool Requirements

### Required
| Tool | Version | Purpose |
|------|---------|---------|
| Rust | 1.75+ | Course exercises |
| Docker | 24+ | Local PostgreSQL, pgvector |
| Docker Compose | v2+ | Multi-container setup |
| Git | 2.x | Clone EdgeQuake repository |

### Optional
| Tool | Purpose |
|------|---------|
| AWS CLI v2 | Cloud deployment chapters (Part III) |
| Node.js 18+ | CDK infrastructure (Chapter 10) |
| Ollama | Local LLM for development |

## Setup

Install Rust if needed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable
```

Clone the EdgeQuake repository:

```bash
git clone https://github.com/edgequake/edgequake.git
cd edgequake
```

Verify your setup:

```bash
cargo --version    # Should show 1.75+
docker --version   # Should show 24+
```

You're ready to begin with [Chapter 1: The RAG Landscape](./part1-foundations/ch01-rag-landscape.md).
