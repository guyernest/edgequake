# 1.1 Why RAG Matters

Large language models are remarkably capable -- and remarkably unreliable in
specific, predictable ways. RAG exists because of three fundamental LLM
failure modes that no amount of scale alone can fix.

## The Three Failure Modes

### 1. Hallucination

LLMs generate fluent, confident text even when they have no grounding in
fact. Ask a model about an obscure internal policy document and it will
cheerfully invent one. This is not a bug -- it is an inherent property of
next-token prediction.

```text
User:  "What is our company's reimbursement policy for conference travel?"
LLM:   "Your company reimburses up to $2,500 per conference including
        airfare, hotel, and registration fees..."

Reality: The model has never seen your company's policy. It hallucinated
         a plausible-sounding answer.
```

Hallucination rates vary by domain, but studies consistently show that even
frontier models hallucinate 3--15% of factual claims in knowledge-intensive
tasks. In regulated industries -- legal, medical, financial -- this is
unacceptable.

### 2. Stale Knowledge

Models have a training cutoff. Events, regulations, product changes, and
market shifts after that date simply do not exist in the model's weights.
Fine-tuning can help but is expensive, slow, and creates its own staleness
problem.

```text
Training cutoff: April 2024
User question:   "What were the key changes in the 2025 Basel IV update?"
LLM response:    Refers to 2023 draft proposals, missing final rules
```

### 3. No Access to Domain Data

This is the most critical limitation for enterprise use. Your organization's
contracts, codebases, customer records, internal wikis, and proprietary
research do not exist in any public training set. The model literally cannot
answer questions about data it has never seen.

## RAG as the Solution

RAG addresses all three failure modes with a single architectural pattern:
**retrieve relevant evidence at query time, then generate an answer grounded
in that evidence.**

```text
┌─────────────────────────────────────────────────────┐
│                    RAG Pipeline                     │
│                                                     │
│  Query ──> Retrieve Evidence ──> Generate Answer    │
│                 │                      │            │
│                 ▼                      ▼            │
│         Your private data      LLM + retrieved     │
│         (always current)       context (grounded)   │
└─────────────────────────────────────────────────────┘
```

| Failure Mode | How RAG Fixes It |
|-------------|-----------------|
| Hallucination | Model generates from retrieved evidence, not parametric memory |
| Stale knowledge | Data store is updated independently of the model |
| No domain data | Your documents are indexed and retrievable |

## RAG vs Fine-Tuning

A common question: "Why not just fine-tune the model on our data?"

| Dimension | RAG | Fine-Tuning |
|-----------|-----|-------------|
| **Data freshness** | Updated in minutes (re-index) | Requires retraining (hours/days) |
| **Cost** | Embedding + storage costs | GPU hours for training |
| **Traceability** | Can cite source documents | No clear provenance |
| **Hallucination control** | Grounded in retrieved text | May still hallucinate |
| **Domain adaptation** | Works with any model | Needs training infrastructure |
| **Data privacy** | Data stays in your infra | Data used in training pipeline |

Fine-tuning teaches a model *how to behave* (style, format, domain
terminology). RAG teaches a model *what to know*. In practice, production
systems often combine both -- fine-tune for style, RAG for knowledge.

## Real-World Use Cases

RAG is not a theoretical nicety. Here are concrete production patterns:

### Legal Document Analysis

A law firm indexes thousands of contracts, court filings, and regulatory
documents. Attorneys query the system to find relevant precedents, clauses,
and obligations. The system returns answers with citations to specific
document sections.

### Internal Knowledge Bases

An engineering organization indexes its runbooks, architecture decision
records, postmortems, and Slack threads. Engineers ask questions like "How
do we handle database failover in the payments service?" and get answers
grounded in actual internal documentation.

### Customer Support

A support platform indexes product documentation, past tickets, and
resolution playbooks. When a customer describes a problem, the system
retrieves relevant solutions and generates a response tailored to the
specific product configuration.

### Financial Research

An investment firm indexes SEC filings, earnings transcripts, and market
reports. Analysts query across thousands of documents to identify trends,
risks, and connections that would take weeks to find manually.

## Why Rust for RAG?

You might wonder why we are building RAG systems in Rust rather than Python.
Several properties of Rust make it well-suited for production RAG:

- **Performance**: Vector similarity search is compute-intensive. Rust's
  zero-cost abstractions and SIMD support make it significantly faster than
  Python for the hot path.
- **Memory safety**: RAG pipelines process untrusted documents. Rust
  eliminates entire classes of memory vulnerabilities.
- **Concurrency**: Production ingestion pipelines process thousands of
  documents concurrently. Rust's async ecosystem (Tokio) handles this
  naturally.
- **Deployment**: A single static binary is simpler to deploy than a Python
  environment with dozens of native dependencies.

```rust
// A taste of what we will build -- similarity search in Rust
// is both fast and expressive
fn top_k_similar(query: &[f32], vectors: &[Vec<f32>], k: usize) -> Vec<usize> {
    let mut scored: Vec<(usize, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (i, cosine_similarity(query, v)))
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scored.into_iter().take(k).map(|(i, _)| i).collect()
}
```

## Summary

RAG exists because LLMs hallucinate, go stale, and cannot see your data.
It solves all three by retrieving evidence at query time and grounding
generation in that evidence. It is cheaper and more flexible than
fine-tuning for knowledge-intensive tasks, and Rust gives us the
performance and safety guarantees that production systems demand.

In the next section we will dissect the full anatomy of a RAG pipeline --
every stage from document ingestion to answer generation.

---

*Next: [1.2 Anatomy of a RAG Pipeline](ch01-02-rag-pipeline.md)*
