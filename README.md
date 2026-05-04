# bacon-lcm

**Lossless Context Memory** — a deterministic, database-backed context management system for LLM agents, modelled after the [Voltropy LCM paper](https://papers.voltropy.com/LCM) and [Volt](https://github.com/Martian-Engineering/volt).

> This is the **Rust port** (`rustic` branch). The original TypeScript implementation lives on `main`.

## Overview

LLM context windows are the primary bottleneck for complex, long-horizon agentic tasks. Even models with 1M+ token windows suffer "context rot" — performance degrades well before the nominal limit is reached.

**bacon-lcm** shifts the burden of memory from the model to a deterministic engine. It maintains:

1. **Immutable Store** — every message is persisted verbatim and never modified
2. **Summary DAG** — a directed acyclic graph of compressed summary nodes that act as materialized views over the history
3. **Deterministic Control Loop** — token-threshold-driven compaction with a three-level escalation protocol

The result: **infinite sessions** with zero information loss and no compaction delays for the end user.

## Architecture

```
┌─────────────────────────────────────────────────┐
│                  LcmSession                     │
│  ┌───────────┐  ┌──────────┐  ┌──────────────┐  │
│  │  Message   │  │ Summary  │  │  Compaction   │  │
│  │  Store     │  │   DAG    │  │   Engine      │  │
│  │ (immutable)│  │ (lineage)│  │ (3-level esc.)│  │
│  └─────┬─────┘  └────┬─────┘  └──────┬───────┘  │
│        │              │               │          │
│        └──────┬───────┴───────┬───────┘          │
│               │               │                  │
│        ┌──────┴─────┐  ┌──────┴──────┐           │
│        │  Context   │  │  Retrieval  │           │
│        │ Assembler  │  │  Service    │           │
│        └────────────┘  └─────────────┘           │
└─────────────────────────────────────────────────┘
```

### Three-Level Escalation Protocol

| Level | Name      | Trigger                  | Mechanism                                      |
|-------|-----------|--------------------------|------------------------------------------------|
| 1     | Leaf      | Soft threshold exceeded  | Groups of raw messages → leaf summary nodes    |
| 2     | Condensed | Still over after Level 1 | Groups of leaf nodes → condensed summary nodes |
| 3     | Emergency | Hard threshold exceeded  | Deterministic archival — no LLM call required  |

### Key Concepts

- **Fresh Tail** — the N most recent raw messages, always kept un-summarized for maximum fidelity
- **Lineage Pointers** — every summary node tracks which messages/nodes it was derived from, enabling lossless expansion
- **`lcm_describe`** — inspect a summary node's metadata (level, archived status, reachable message count)
- **`lcm_expand`** — follow lineage pointers to retrieve the original verbatim messages

## Workspace Layout

```
bacon-lcm/
  core/           Core library: session, compaction, storage traits, providers
  daemon/         HTTP service: /health, /metrics (Prometheus), /status + Postgres storage
  mcp-server/     MCP server exposing LCM as six tools (stdio transport)
  cli/            Developer CLI: dev, test, bench, migrate subcommands
  docker/         Dockerfile + docker-compose for the full stack
  docs/           Design specs and implementation plans
```

## Quick Start

### Cargo (library)

```bash
git clone https://github.com/bacon-lcm/bacon-lcm
cd bacon-lcm
git checkout rustic
cargo test --workspace --lib
```

### Docker (full stack)

```bash
cd docker
docker compose up
```

Services started:
- **postgres** — pgvector/pgvector:pg16, port 5432
- **daemon** — HTTP server on port 3333 (`/health`, `/metrics`, `/status`)
- **mcp-server** — stdio MCP server connected to Postgres

### CLI

```bash
cargo run -p bacon-lcm-cli -- --help
```

```
Commands:
  dev      Start a local development session
  test     Run test suites
  bench    Run performance benchmarks
  migrate  Migrate data between databases
```

## Usage (Rust library)

```rust
use bacon_lcm_core::{
    LcmConfig,
    providers::{create_token_counter, create_summarizer, create_embedder},
    storage::StorageLayer,
    session::LcmSession,
    types::MessageRole,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let token_counter = create_token_counter("naive", None)?;
    let summarizer    = create_summarizer("echo", "echo".to_string(), None, None, None, None)?;
    let embedder      = create_embedder("null", None, None, None, None)?;

    let mut session = LcmSession::new(
        token_counter,
        summarizer,
        embedder,
        LcmConfig::defaults(),
        StorageLayer::memory(),
    )
    .await?;

    // Add messages — compaction runs automatically when thresholds are exceeded
    session.add_message(MessageRole::User,      "Explain quantum computing".into()).await?;
    session.add_message(MessageRole::Assistant, "Quantum computing uses qubits...".into()).await?;

    // Active context window (summaries + fresh tail)
    let context = session.get_context().await?;

    // Session statistics
    let info = session.get_session_info().await?;
    println!("messages: {}, tokens: {}", info.message_count, info.token_count);

    // Inspect a summary node
    // let desc = session.describe(summary_id).await?;

    // Expand back to original messages
    // let messages = session.expand(summary_id).await?;

    Ok(())
}
```

## Pluggable Providers

| Interface      | Purpose                         | Implementations                                         |
|----------------|---------------------------------|---------------------------------------------------------|
| `TokenCounter` | Estimate token count for text   | `NaiveTokenCounter`, `TiktokenCounter`, `AnthropicTokenCounter` |
| `Summarizer`   | LLM call to produce summaries   | `EchoSummarizer`, `OpenAISummarizer`, `AnthropicSummarizer`     |
| `Embedder`     | Generate embedding vectors      | `NullEmbedder`, `OpenAIEmbedder`, `LocalEmbedder`               |
| `MessageStore` | Persistence for raw messages    | `InMemoryMessageStore`, Postgres (via daemon crate)             |
| `SummaryDag`   | Persistence for the summary DAG | `InMemorySummaryDag`, Postgres (via daemon crate)               |

Providers are selected at runtime via factory functions:

```rust
// Summarizer providers: "echo" | "openai" | "anthropic"
let summarizer = create_summarizer("openai", "gpt-4o-mini".into(), None, Some(api_key), None, None)?;

// Token counter: "naive" | "tiktoken" | "anthropic"
let counter = create_token_counter("tiktoken", Some("gpt-4o"))?;

// Embedder: "null" | "openai" | "local"
let embedder = create_embedder("openai", Some("text-embedding-3-small"), None, Some(api_key), None)?;
```

## PostgreSQL Persistence

Run the daemon (which owns the Postgres schema) or apply migrations directly:

```bash
# Via the daemon (auto-migrates on startup)
DATABASE_URL=postgres://localhost:5432/bacon_lcm cargo run -p bacon-lcm-daemon

# Via Docker Compose (recommended)
cd docker && docker compose up
```

Use Postgres storage in code:

```rust
use bacon_lcm_daemon::storage::postgres_layer;

let pool = sqlx::PgPool::connect(&database_url).await?;
let storage = postgres_layer(pool);

let session = LcmSession::new(token_counter, summarizer, embedder, config, storage).await?;
```

Migrations are in `daemon/migrations/`:
- `0001_init.sql` — sessions, messages, summary nodes tables
- `0002_embeddings.sql` — pgvector embeddings table + indexes

## MCP Server

The MCP server exposes LCM as six tools over stdio. Any MCP-compatible agent can connect to it.

### Run directly

```bash
# In-memory (no database)
cargo run -p bacon-lcm-mcp-server

# With Postgres
DATABASE_URL=postgres://localhost:5432/bacon_lcm \
LCM_SUMMARIZER_PROVIDER=openai \
LCM_SUMMARIZER_MODEL=gpt-4o-mini \
LCM_SUMMARIZER_API_KEY=$OPENAI_API_KEY \
cargo run -p bacon-lcm-mcp-server
```

### Windsurf

Add to `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "bacon-lcm": {
      "command": "cargo",
      "args": ["run", "--manifest-path", "/path/to/bacon-lcm/Cargo.toml", "-p", "bacon-lcm-mcp-server"],
      "env": {
        "DATABASE_URL": "postgres://localhost:5432/bacon_lcm",
        "LCM_SUMMARIZER_PROVIDER": "openai",
        "LCM_SUMMARIZER_MODEL": "gpt-4o-mini",
        "LCM_SUMMARIZER_API_KEY": "<your-key>"
      }
    }
  }
}
```

Or use the pre-built Docker image:

```json
{
  "mcpServers": {
    "bacon-lcm": {
      "command": "docker",
      "args": ["compose", "-f", "/path/to/bacon-lcm/docker/docker-compose.yml", "run", "--rm", "mcp-server"],
      "env": { "DATABASE_URL": "postgres://bacon_lcm:bacon_lcm@postgres:5432/bacon_lcm" }
    }
  }
}
```

### MCP Tools

| Tool              | Description                                              |
|-------------------|----------------------------------------------------------|
| `lcm_store`       | Persist a message; auto-compaction when thresholds exceeded |
| `lcm_recall`      | Retrieve active context window (summaries + fresh tail)  |
| `lcm_describe`    | Inspect a summary node's lineage metadata                |
| `lcm_expand`      | Expand a summary to original verbatim messages           |
| `lcm_session_new` | Create a new LCM session                                 |
| `lcm_session_info`| Get current session statistics                           |

### MCP Server Environment Variables

| Variable                   | Default    | Description                                    |
|----------------------------|------------|------------------------------------------------|
| `DATABASE_URL`             | *(none)*   | Postgres URL; falls back to in-memory if unset |
| `LCM_SUMMARIZER_PROVIDER`  | `echo`     | `echo` / `openai` / `anthropic`                |
| `LCM_SUMMARIZER_MODEL`     | `echo`     | Model name                                     |
| `LCM_SUMMARIZER_API_KEY`   | *(none)*   | API key (required for openai / anthropic)      |
| `RUST_LOG`                 | `info`     | Log filter (tracing-subscriber)                |

## Daemon HTTP API

The daemon provides an HTTP server for health checks, observability, and status.

```bash
# Start (DATABASE_URL required)
DATABASE_URL=postgres://localhost:5432/bacon_lcm cargo run -p bacon-lcm-daemon

# Or via Docker
cd docker && docker compose up daemon
```

### Daemon Environment Variables

| Variable       | Default | Description                           |
|----------------|---------|---------------------------------------|
| `DATABASE_URL` | *(required)* | PostgreSQL connection string     |
| `LCM_PORT`     | `3333`  | HTTP server port                      |
| `RUST_LOG`     | `info`  | Log filter                            |

### Endpoints

```
GET /health    — DB ping; 200 OK or 503 with error
GET /metrics   — Prometheus text format (messages_stored, compaction_runs, active_sessions, token_counts)
GET /status    — JSON snapshot of daemon state
```

```bash
curl http://localhost:3333/health
curl http://localhost:3333/metrics
curl http://localhost:3333/status
```

## CLI

```bash
# Start an in-memory dev session (prints session ID + stats)
cargo run -p bacon-lcm-cli -- dev

# With Postgres
cargo run -p bacon-lcm-cli -- dev --database-url postgres://localhost:5432/bacon_lcm

# Run unit tests
cargo run -p bacon-lcm-cli -- test

# Run property-based tests (proptest)
cargo run -p bacon-lcm-cli -- test --property

# Run integration tests (requires Docker/Postgres)
cargo run -p bacon-lcm-cli -- test --integration

# Run Criterion benchmarks
cargo run -p bacon-lcm-cli -- bench
cargo run -p bacon-lcm-cli -- bench --export results.txt

# Migrate data between two Postgres databases
cargo run -p bacon-lcm-cli -- migrate \
  --from-url postgres://old-host:5432/bacon_lcm \
  --to-url   postgres://new-host:5432/bacon_lcm \
  --dry-run
```

## Testing

```bash
# All unit tests
cargo test --workspace --lib

# Property-based tests (proptest)
cargo test -p bacon-lcm-core --test property_tests

# MCP server smoke test
cargo test -p bacon-lcm-mcp-server --test smoke_test

# Criterion benchmarks (compile + single-iteration test run)
cargo test --benches -p bacon-lcm-core

# Full Criterion benchmark run (produces HTML report in target/criterion/)
cargo bench -p bacon-lcm-core
```

| Suite                                | Tests |
|--------------------------------------|-------|
| `bacon-lcm-core` (unit)              | 81    |
| `bacon-lcm-core` (proptest)          | 3     |
| `bacon-lcm-daemon` (unit)            | 8     |
| `bacon-lcm-mcp-server` (unit)        | 14    |
| `bacon-lcm-mcp-server` (smoke test)  | 1     |
| **Total**                            | **107** |

### Property Tests

Three invariants are verified with randomised inputs via proptest:

1. **`session_counts_consistent`** — after adding N messages, at least one message or summary exists and token count is accessible
2. **`context_items_ordered_by_timestamp`** — items returned by `get_context()` are in non-decreasing timestamp order
3. **`token_count_positive_after_messages`** — adding non-empty messages always produces a positive token count

### Benchmarks

Three Criterion benchmarks in `core/benches/compaction.rs`:

| Benchmark                        | What it measures                              |
|----------------------------------|-----------------------------------------------|
| `leaf_compaction_20_messages`    | Full compaction cycle with tight thresholds   |
| `get_context_100_messages`       | Context assembly over 100 un-compacted msgs   |
| `add_message_throughput_500`     | Raw message ingestion rate (no compaction)    |

## Configuration

bacon-lcm uses a layered config system in `LcmConfig`: **defaults ← programmatic override**.

The default compaction thresholds:

| Setting              | Default   |
|----------------------|-----------|
| `model_max_tokens`   | 128 000   |
| `soft_limit`         | 80 000    |
| `hard_limit`         | 110 000   |
| `fresh_tail_count`   | 10        |
| `leaf_group_size`    | 20        |
| `condensed_group_size` | 10      |

Override at construction time:

```rust
use bacon_lcm_core::types::{CompactionConfig, ThresholdConfig};

let mut config = LcmConfig::defaults();
config.compaction = CompactionConfig {
    thresholds: ThresholdConfig {
        model_max_tokens: 200_000,
        soft_limit:       150_000,
        hard_limit:       180_000,
    },
    fresh_tail_count: 20,
    leaf_group_size: 30,
    condensed_group_size: 10,
    parallel_compaction: true,
    max_concurrent_compactions: 4,
};
```

## References

- [LCM: Lossless Context Management](https://papers.voltropy.com/LCM) — Clint Ehrlich, Voltropy PBC
- [Volt](https://github.com/Martian-Engineering/volt) — Coding agent with lossless context management
- [losslesscontext.ai](https://www.losslesscontext.ai/) — Visual explainer
