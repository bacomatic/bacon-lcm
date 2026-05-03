# Rustic LCM Design Specification

**Project**: Port bacon-lcm from TypeScript to Rust  
**Branch**: `rustic`  
**Date**: 2025-05-03  
**Status**: Design Complete  

## Overview

This specification outlines the complete rewrite of **bacon-lcm** (Lossless Context Memory) from TypeScript to Rust, focusing on performance improvements, deployment simplicity, safety guarantees, and enhanced developer experience while maintaining backward compatibility.

### Motivation

1. **Performance** - Faster compaction, lower memory usage, better concurrency
2. **Deployment Simplicity** - Docker-first approach with optimized single binary
3. **Safety Guarantees** - Memory safety for long-running daemon processes
4. **Developer Experience** - Rich host-side tooling for local development
5. **Learning Opportunity** - Exploring Rust's ecosystem for LLM tooling

## Architecture

### High-Level Design

```
┌─────────────────────────────────────────────────┐
│                Docker Container                 │
│  ┌─────────────┐  ┌─────────────┐  ┌──────────┐  │
│  │   MCP       │  │   Daemon    │  │  Web     │  │
│  │  Server     │  │  Process    │  │ Dashboard│  │
│  └──────┬──────┘  └──────┬──────┘  └────┬─────┘  │
│         │                 │              │        │
│  ┌──────┴─────────────────┴──────────────┴─────┐  │
│  │              Core LCM Library               │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  │  │
│  │  │ Session  │  │Compaction│  │ Storage  │  │  │
│  │  │ Manager  │  │  Engine   │  │ Layer    │  │  │
│  │  └──────────┘  └──────────┘  └──────────┘  │  │
│  └───────────────────────────────────────────────┘
└─────────────────────────────────────────────────┘
                    │
            ┌───────┴───────┐
            │ Host CLI Tools│
            │  dev/test/    │
            │  bench/migrate│
            └───────────────┘
```

### Project Structure

```
bacon-lcm-rust/
├── Cargo.toml                    # Workspace configuration
├── docker/
│   ├── Dockerfile                # Multi-stage optimized build
│   ├── docker-compose.yml        # Development environment
│   └── entrypoint.sh             # Health checks & signal handling
├── cli/                          # Host-side development tools
│   ├── Cargo.toml
│   └── src/
│       ├── dev.rs                # Local development commands
│       ├── test.rs               # Test suite runner
│       ├── bench.rs              # Performance benchmarks
│       └── migrate.rs            # Data migration tools
├── core/                         # Core LCM library
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                # Public API surface
│       ├── session/              # Session management
│       ├── compaction/           # Three-level compaction engine
│       ├── storage/              # Persistence layer
│       ├── providers/            # LLM/embedding providers
│       └── types.rs              # Core type definitions
├── mcp-server/                   # MCP server binary
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs               # Server entry point
│       └── handlers/             # MCP tool implementations
├── daemon/                       # Background daemon binary
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs               # Daemon entry point
│       └── service.rs            # Long-running service logic
├── tests/                        # Integration and property tests
│   ├── integration/              # End-to-end tests
│   ├── property/                 # Proptest-based correctness tests
│   └── fixtures/                 # Test data and migrations
├── benches/                      # Performance benchmarks
│   ├── compaction/               # Compaction performance
│   ├── storage/                  # Database operation benchmarks
│   └── providers/                # LLM provider latency
├── sql/                          # Database migrations (shared with TS)
│   ├── 001_init.sql
│   └── 002_embeddings.sql
└── README.md
```

## Core Components

### 1. Core Library (`core/`)

#### Dependencies
- **Runtime**: `tokio` for async runtime
- **Database**: `sqlx` with compile-time checked queries
- **Serialization**: `serde` for config and data structures
- **HTTP**: `reqwest` for LLM provider communication
- **Logging**: `tracing` for structured logging
- **Error Handling**: `anyhow` + `thiserror` for error types

#### Key Traits

```rust
// Token counting abstraction
pub trait TokenCounter: Send + Sync {
    async fn count(&self, text: &str) -> Result<usize, TokenError>;
}

// LLM summarization abstraction
pub trait Summarizer: Send + Sync {
    async fn summarize(&self, messages: &[Message]) -> Result<String, SummarizerError>;
}

// Embedding generation abstraction
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedderError>;
}

// Persistence abstractions
pub trait MessageStore: Send + Sync {
    async fn store(&self, message: Message) -> Result<MessageId, StoreError>;
    async fn get(&self, id: MessageId) -> Result<Option<Message>, StoreError>;
    async fn get_range(&self, session_id: SessionId, range: Range<usize>) -> Result<Vec<Message>, StoreError>;
}

pub trait SummaryDag: Send + Sync {
    async fn add_node(&self, node: SummaryNode) -> Result<SummaryId, DagError>;
    async fn get_lineage(&self, id: SummaryId) -> Result<Vec<SummaryId>, DagError>;
    async fn expand(&self, id: SummaryId) -> Result<Vec<Message>, DagError>;
}

pub trait SessionStore: Send + Sync {
    async fn save(&self, session: Session) -> Result<(), StoreError>;
    async fn load(&self, id: SessionId) -> Result<Option<Session>, StoreError>;
    async fn list(&self) -> Result<Vec<Session>, StoreError>;
}

pub trait VectorStore: Send + Sync {
    async fn store_embedding(&self, message_id: MessageId, embedding: Vec<f32>) -> Result<(), StoreError>;
    async fn search(&self, query: Vec<f32>, limit: usize) -> Result<Vec<SearchResult>, StoreError>;
}
```

#### Performance Optimizations

1. **Lock-free Data Structures**: Use `crossbeam` for message store operations
2. **Parallel Compaction**: Work-stealing for multi-level compaction
3. **Zero-copy Handling**: `Cow<str>` for message text where possible
4. **Connection Pooling**: Optimized SQLx connection pools
5. **Memory Management**: Arena allocation for batch operations

### 2. Provider System (`core/providers/`)

#### Implementation Strategy

**Hybrid Approach**: Use ecosystem crates for major providers, maintain extensible trait system.

```rust
// Provider registry
pub struct ProviderRegistry {
    summarizers: HashMap<String, Box<dyn Summarizer>>,
    embedders: HashMap<String, Box<dyn Embedder>>,
    token_counters: HashMap<String, Box<dyn TokenCounter>>,
}

// Built-in providers
pub mod openai {
    pub struct OpenAISummarizer { /* uses async-openai crate */ }
    pub struct OpenAIEmbedder { /* uses async-openai crate */ }
    pub struct TiktokenCounter { /* uses tiktoken-rs crate */ }
}

pub mod anthropic {
    pub struct AnthropicSummarizer { /* uses anthropic-rs crate */ }
    pub struct AnthropicTokenCounter { /* custom implementation */ }
}

pub mod local {
    pub struct LocalEmbedder { /* uses candle/ort for local models */ }
}
```

#### Configuration Compatibility

Support existing TypeScript config format with Rust-specific extensions:

```json
{
  "summarizer": {
    "provider": "openai",
    "model": "gpt-4o-mini",
    "baseUrl": "https://api.openai.com/v1",
    "rust": {
      "max_concurrent_requests": 10,
      "request_timeout_ms": 30000,
      "retry_policy": "exponential"
    }
  },
  "compaction": {
    "thresholds": { "modelMaxTokens": 128000, "softLimit": 80000, "hardLimit": 110000 },
    "freshTailCount": 10,
    "rust": {
      "parallel_compaction": true,
      "compaction_workers": 4,
      "memory_limit_mb": 512
    }
  }
}
```

### 3. Storage Layer (`core/storage/`)

#### PostgreSQL Implementation

```rust
pub struct PgMessageStore {
    pool: PgPool,
    token_counter: Arc<dyn TokenCounter>,
}

pub struct PgSummaryDag {
    pool: PgPool,
    token_counter: Arc<dyn TokenCounter>,
}

pub struct PgSessionStore {
    pool: PgPool,
}

pub struct PgVectorStore {
    pool: PgPool,
    dimensions: usize,
}
```

#### Migration Strategy

- **Shared SQL**: Use existing migration files from TypeScript version
- **Automatic Upgrades**: Detect and apply schema changes on startup
- **Backward Compatibility**: Support older schema versions with feature flags

### 4. Compaction Engine (`core/compaction/`)

#### Three-Level Escalation Protocol

```rust
pub struct CompactionEngine {
    config: CompactionConfig,
    message_store: Arc<dyn MessageStore>,
    summary_dag: Arc<dyn SummaryDag>,
    summarizer: Arc<dyn Summarizer>,
}

#[derive(Debug, Clone)]
pub enum CompactionLevel {
    Leaf { group_size: usize },
    Condensed { group_size: usize },
    Emergency { archive_all: bool },
}

impl CompactionEngine {
    pub async fn compact(&self, session_id: SessionId) -> Result<CompactionResult, CompactionError> {
        let token_count = self.get_token_count(session_id).await?;
        
        match self.determine_compaction_level(token_count) {
            CompactionLevel::Leaf { group_size } => {
                self.leaf_compaction(session_id, group_size).await
            }
            CompactionLevel::Condensed { group_size } => {
                self.condensed_compaction(session_id, group_size).await
            }
            CompactionLevel::Emergency { archive_all } => {
                self.emergency_compaction(session_id, archive_all).await
            }
        }
    }
}
```

#### Parallel Compaction

```rust
impl CompactionEngine {
    async fn parallel_leaf_compaction(&self, groups: Vec<Vec<MessageId>>) -> Result<Vec<SummaryId>, CompactionError> {
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_compactions));
        let tasks: Vec<_> = groups
            .into_iter()
            .map(|group| {
                let semaphore = Arc::clone(&semaphore);
                let message_store = Arc::clone(&self.message_store);
                let summarizer = Arc::clone(&self.summarizer);
                
                tokio::spawn(async move {
                    let _permit = semaphore.acquire().await?;
                    let messages = message_store.get_batch(group).await?;
                    let summary = summarizer.summarize(&messages).await?;
                    Ok(summary)
                })
            })
            .collect();
        
        let results = try_join_all(tasks).await?;
        Ok(results.into_iter().collect::<Result<Vec<_>, _>>()?)
    }
}
```

## Docker-First Deployment

### Multi-Stage Dockerfile

```dockerfile
# Build stage
FROM rust:1.75-slim as builder
WORKDIR /app
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/lcm-mcp /usr/local/bin/
COPY --from=builder /app/target/release/lcm-daemon /usr/local/bin/
COPY --from=builder /app/sql/ /sql/
EXPOSE 3333
ENTRYPOINT ["/usr/local/bin/lcm-daemon"]
```

### Health Checks & Signal Handling

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Graceful shutdown handling
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = shutdown_tx.send(true);
    });
    
    // Health check endpoint
    let health_check = warp::path("health")
        .and(warp::get())
        .map(|| warp::reply::json(&json!({"status": "healthy"})));
    
    // Main service
    let service = run_service(shutdown_rx.clone());
    
    tokio::select! {
        _ = service => {},
        _ = shutdown_rx.changed() => {
            tracing::info!("Received shutdown signal, gracefully shutting down");
        }
    }
    
    Ok(())
}
```

## Host-Side CLI Tools

### Development Tools (`cli/`)

#### `lcm-cli dev` - Local Development

```rust
// cli/src/dev.rs
pub struct DevCommand {
    #[arg(short, long)]
    watch: bool,
    #[arg(short, long)]
    database_url: Option<String>,
}

impl DevCommand {
    pub async fn run(self) -> Result<(), CliError> {
        if self.watch {
            // Hot reload development server
            let mut watcher = notify::recommended_watcher(move |_| {
                // Rebuild and restart services
            })?;
            watcher.watch(Path::new("src"), RecursiveMode::Recursive)?;
        }
        
        // Start development environment
        start_dev_environment(self.database_url).await?;
        Ok(())
    }
}
```

#### `lcm-cli test` - Comprehensive Testing

```rust
// cli/src/test.rs
pub struct TestCommand {
    #[arg(short, long)]
    integration: bool,
    #[arg(short, long)]
    property: bool,
    #[arg(short, long)]
    benchmark: bool,
}

impl TestCommand {
    pub async fn run(self) -> Result<(), CliError> {
        if self.integration {
            run_integration_tests().await?;
        }
        if self.property {
            run_property_tests().await?;
        }
        if self.benchmark {
            run_benchmarks().await?;
        }
        Ok(())
    }
}
```

#### `lcm-cli bench` - Performance Comparison

```rust
// cli/src/bench.rs
pub struct BenchCommand {
    #[arg(short, long)]
    compare: bool,  // Compare with TypeScript version
    #[arg(short, long)]
    export: Option<PathBuf>,  // Export results
}

impl BenchCommand {
    pub async fn run(self) -> Result<(), CliError> {
        let rust_results = run_rust_benchmarks().await?;
        
        if self.compare {
            let ts_results = run_typescript_benchmarks().await?;
            print_comparison(&rust_results, &ts_results);
        }
        
        if let Some(export_path) = self.export {
            export_results(&rust_results, export_path).await?;
        }
        
        Ok(())
    }
}
```

#### `lcm-cli migrate` - Data Migration

```rust
// cli/src/migrate.rs
pub struct MigrateCommand {
    #[arg(short, long)]
    from_url: String,  // TypeScript PostgreSQL URL
    #[arg(short, long)]
    to_url: String,    // Rust PostgreSQL URL
    #[arg(short, long)]
    dry_run: bool,
}

impl MigrateCommand {
    pub async fn run(self) -> Result<(), CliError> {
        let migration_plan = analyze_migration(&self.from_url, &self.to_url).await?;
        
        if self.dry_run {
            print_migration_plan(&migration_plan);
            return Ok(());
        }
        
        execute_migration(migration_plan).await?;
        Ok(())
    }
}
```

## Testing Strategy

### Comprehensive Test Suite

#### 1. Unit Tests
- Core component logic
- Provider implementations
- Configuration parsing
- Error handling paths

#### 2. Integration Tests
- End-to-end session workflows
- Database operations with testcontainers
- MCP server protocol compliance
- Docker container functionality

#### 3. Property-Based Tests

```rust
// tests/property/compaction.rs
use proptest::prelude::*;

proptest! {
    #[test]
    fn compaction_preserves_information(
        messages in prop::collection::vec(any::<Message>(), 10..1000)
    ) {
        // Property: Compaction never loses information
        let session = create_test_session().await;
        
        // Add all messages
        for msg in messages {
            session.add_message(msg).await.unwrap();
        }
        
        // Get all summaries and expand them
        let summaries = session.get_summaries().await;
        let expanded = session.expand_all(summaries).await;
        
        // Property: All original messages should be recoverable
        prop_assert_eq!(expanded.len(), messages.len());
    }
    
    #[test]
    fn token_count_monotonicity(
        operations in prop::collection::vec(any::<CompactionOperation>(), 1..100)
    ) {
        // Property: Token count never increases during compaction
        let session = create_test_session().await;
        let mut initial_count = 0;
        
        for op in operations {
            initial_count = session.get_token_count().await;
            session.apply_operation(op).await.unwrap();
            let new_count = session.get_token_count().await;
            prop_assert!(new_count <= initial_count);
        }
    }
}
```

#### 4. Performance Benchmarks

```rust
// benches/compaction.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_compaction_performance(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    
    c.bench_function("leaf_compaction_1000_messages", |b| {
        b.to_async(&rt).iter(|| async {
            let session = create_benchmark_session(1000).await;
            session.compact().await.unwrap();
        })
    });
    
    c.bench_function("parallel_vs_sequential_compaction", |b| {
        b.to_async(&rt).iter(|| async {
            let session = create_benchmark_session(5000).await;
            session.parallel_compact().await.unwrap();
        })
    });
}

criterion_group!(benches, bench_compaction_performance);
criterion_main!(benches);
```

## MCP Server Integration

### Using Existing MCP Crate

```rust
// mcp-server/src/main.rs
use model_context_protocol::{Server, Tool};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = Server::new("bacon-lcm", "1.0.0");
    
    // Register LCM tools
    server.add_tool(Tool::new("lcm_store", "Store a message in LCM", store_message_handler));
    server.add_tool(Tool::new("lcm_recall", "Retrieve active context", recall_context_handler));
    server.add_tool(Tool::new("lcm_describe", "Inspect summary metadata", describe_summary_handler));
    server.add_tool(Tool::new("lcm_expand", "Expand summary to original messages", expand_summary_handler));
    server.add_tool(Tool::new("lcm_session_new", "Create new session", create_session_handler));
    server.add_tool(Tool::new("lcm_session_info", "Get session statistics", session_info_handler));
    
    // Start server
    server.run_stdio().await?;
    Ok(())
}
```

### Enhanced MCP Features

```rust
// Rust-specific enhancements
async fn bulk_store_messages_handler(args: BulkStoreArgs) -> Result<StoreResult, McpError> {
    // Bulk operation for better performance
    let session = get_active_session().await?;
    let results = session.add_messages_batch(args.messages).await?;
    Ok(StoreResult { stored_count: results.len() })
}

async fn stream_context_handler(args: StreamArgs) -> Result<impl Stream<Item = ContextChunk>, McpError> {
    // Streaming responses for large contexts
    let session = get_active_session().await?;
    Ok(session.stream_context(args.chunk_size).await?)
}
```

## Configuration System

### Enhanced Compatibility

```rust
// core/src/config.rs
#[derive(Debug, Clone, Deserialize)]
pub struct LcmConfig {
    pub summarizer: SummarizerConfig,
    pub compaction: CompactionConfig,
    pub database_url: Option<String>,
    pub dashboard: Option<DashboardConfig>,
    pub rust: Option<RustSpecificConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RustSpecificConfig {
    pub max_concurrent_requests: Option<usize>,
    pub request_timeout_ms: Option<u64>,
    pub retry_policy: Option<RetryPolicy>,
    pub parallel_compaction: Option<bool>,
    pub compaction_workers: Option<usize>,
    pub memory_limit_mb: Option<usize>,
}

impl LcmConfig {
    pub fn load() -> Result<Self, ConfigError> {
        // 1. Load defaults
        let mut config = Self::defaults();
        
        // 2. Override with config file (if exists)
        if let Some(file_config) = Self::load_from_file()? {
            config.merge(file_config);
        }
        
        // 3. Override with environment variables
        config.merge_env();
        
        // 4. Validate configuration
        config.validate()?;
        
        Ok(config)
    }
}
```

### Environment Variable Support

Maintain full compatibility with existing TypeScript environment variables:

```rust
impl LcmConfig {
    fn merge_env(&mut self) {
        if let Ok(provider) = env::var("LCM_SUMMARIZER_PROVIDER") {
            self.summarizer.provider = provider;
        }
        
        if let Ok(model) = env::var("LCM_SUMMARIZER_MODEL") {
            self.summarizer.model = model;
        }
        
        // Rust-specific environment variables
        if let Ok(max_requests) = env::var("LCM_RUST_MAX_CONCURRENT_REQUESTS") {
            self.rust.get_or_insert_with(Default::default)
                .max_concurrent_requests = Some(max_requests.parse().unwrap());
        }
    }
}
```

## Migration Strategy

### Data Migration Tools

```rust
// cli/src/migrate.rs
pub struct MigrationPlan {
    pub sessions_to_migrate: Vec<SessionMigration>,
    pub estimated_time: Duration,
    pub required_disk_space: u64,
    pub compatibility_issues: Vec<CompatibilityIssue>,
}

pub async fn analyze_migration(from_url: &str, to_url: &str) -> Result<MigrationPlan, MigrationError> {
    let from_pool = PgPoolOptions::new().connect(from_url).await?;
    let to_pool = PgPoolOptions::new().connect(to_url).await?;
    
    // Analyze existing data
    let sessions = query_sessions(&from_pool).await?;
    let migration_plan = plan_migration(sessions).await?;
    
    Ok(migration_plan)
}

pub async fn execute_migration(plan: MigrationPlan) -> Result<MigrationResult, MigrationError> {
    let mut progress = MigrationProgress::new(plan.sessions_to_migrate.len());
    
    for session_plan in plan.sessions_to_migrate {
        migrate_session(session_plan).await?;
        progress.increment();
    }
    
    Ok(MigrationResult { 
        migrated_sessions: progress.completed(),
        duration: progress.elapsed(),
    })
}
```

### Compatibility Validation

```rust
// tests/migration/compatibility.rs
#[tokio::test]
async fn test_typescript_rust_compatibility() {
    // Create data with TypeScript version
    let ts_session = create_typescript_session().await;
    ts_session.add_test_messages().await;
    
    // Migrate to Rust version
    let rust_session = migrate_session_to_rust(ts_session.id()).await;
    
    // Validate data integrity
    assert_eq!(ts_session.get_token_count(), rust_session.get_token_count());
    assert_eq!(ts_session.get_message_count(), rust_session.get_message_count());
    
    // Validate compaction results
    let ts_context = ts_session.get_context().await;
    let rust_context = rust_session.get_context().await;
    assert_context_equivalent(ts_context, rust_context);
}
```

## Performance Targets

### Benchmark Goals

| Metric | TypeScript Baseline | Rust Target | Improvement |
|--------|-------------------|-------------|-------------|
| Message Storage | 1000 msg/s | 5000 msg/s | 5x |
| Compaction Speed | 100 msg/s | 500 msg/s | 5x |
| Memory Usage | 100MB | 50MB | 2x reduction |
| Startup Time | 2s | 0.5s | 4x faster |
| Concurrent Sessions | 10 | 100 | 10x |

### Monitoring and Metrics

```rust
// core/src/metrics.rs
use prometheus::{Counter, Histogram, Gauge, IntGauge};

lazy_static! {
    static ref MESSAGES_STORED: Counter = Counter::new(
        "lcm_messages_stored_total", "Total messages stored"
    ).unwrap();
    
    static ref COMPACTION_DURATION: Histogram = Histogram::with_opts(
        prometheus::HistogramOpts::new("lcm_compaction_duration_seconds", "Compaction duration")
            .buckets(vec![0.1, 0.5, 1.0, 2.0, 5.0])
    ).unwrap();
    
    static ref ACTIVE_SESSIONS: IntGauge = IntGauge::new(
        "lcm_active_sessions", "Number of active sessions"
    ).unwrap();
    
    static ref MEMORY_USAGE: Gauge = Gauge::new(
        "lcm_memory_usage_bytes", "Memory usage in bytes"
    ).unwrap();
}
```

## Security Considerations

### API Key Management

```rust
// core/src/security.rs
pub struct SecureConfig {
    pub api_keys: HashMap<String, SecretString>,
    pub encryption_key: SecretString,
}

impl SecureConfig {
    pub fn load() -> Result<Self, SecurityError> {
        // Load from environment variables or encrypted config
        let api_keys = Self::load_api_keys()?;
        let encryption_key = Self::load_or_generate_key()?;
        
        Ok(Self { api_keys, encryption_key })
    }
    
    fn load_api_keys() -> Result<HashMap<String, SecretString>, SecurityError> {
        let mut keys = HashMap::new();
        
        if let Ok(openai_key) = env::var("OPENAI_API_KEY") {
            keys.insert("openai".to_string(), SecretString::new(openai_key));
        }
        
        if let Ok(anthropic_key) = env::var("ANTHROPIC_API_KEY") {
            keys.insert("anthropic".to_string(), SecretString::new(anthropic_key));
        }
        
        Ok(keys)
    }
}
```

### Database Security

```rust
// core/src/storage/pg.rs
impl PgMessageStore {
    pub async fn new_with_ssl(database_url: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect_with(
                database_url.parse::<PgConnectOptions>()?
                    .ssl_mode(PgSslMode::Require)
            )
            .await?;
            
        Ok(Self { pool, token_counter: /* ... */ })
    }
}
```

## Error Handling Strategy

### Structured Error Types

```rust
// core/src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LcmError {
    #[error("Storage error: {0}")]
    Storage(#[from] StoreError),
    
    #[error("Compaction error: {0}")]
    Compaction(#[from] CompactionError),
    
    #[error("Provider error: {0}")]
    Provider(#[from] ProviderError),
    
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),
    
    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),
    
    #[error("Token limit exceeded: {current}/{max}")]
    TokenLimitExceeded { current: usize, max: usize },
}

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Database connection failed: {0}")]
    ConnectionFailed(#[from] sqlx::Error),
    
    #[error("Message not found: {0}")]
    MessageNotFound(MessageId),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
```

### Graceful Degradation

```rust
// core/src/session.rs
impl LcmSession {
    pub async fn add_message_with_fallback(&mut self, role: MessageRole, content: String) -> Result<MessageId, LcmError> {
        // Try to store with full features
        match self.add_message(role, content.clone()).await {
            Ok(id) => Ok(id),
            
            // Fallback strategies
            Err(LcmError::Provider(ProviderError::RateLimited)) => {
                // Store without embedding generation
                self.add_message_no_embedding(role, content).await
            }
            
            Err(LcmError::Storage(StoreError::ConnectionFailed)) => {
                // Fall back to in-memory storage temporarily
                self.add_message_in_memory(role, content).await
            }
            
            Err(e) => Err(e),
        }
    }
}
```

## Documentation Strategy

### Code Documentation

- **Rustdoc**: Comprehensive API documentation with examples
- **Architecture Docs**: High-level design decisions and trade-offs
- **Migration Guide**: Step-by-step migration from TypeScript version
- **Performance Guide**: Tuning recommendations and benchmarking

### Developer Documentation

```markdown
# Development Guide

## Quick Start
```bash
# Clone repository
git clone https://github.com/your-org/bacon-lcm-rust
cd bacon-lcm-rust

# Setup development environment
cargo install cargo-watch sqlx-cli
docker-compose up -d postgres

# Run tests
cargo test

# Start development server
cargo run --bin lcm-daemon
```

## Architecture Overview
[Detailed architecture documentation]

## Performance Profiling
[Profiling tools and techniques]

## Contributing
[Contribution guidelines and code standards]
```

## Rollout Plan

### Phase 1: Core Engine (Weeks 1-4)
- Implement core types and traits
- Build PostgreSQL storage layer
- Implement three-level compaction engine
- Add comprehensive unit and property tests

### Phase 2: Provider Integration (Weeks 5-6)
- Implement OpenAI and Anthropic providers
- Add embedding support
- Create configuration system
- Test provider compatibility

### Phase 3: Server Components (Weeks 7-8)
- Build MCP server using existing crate
- Implement daemon process
- Add health checks and monitoring
- Create Docker image

### Phase 4: Developer Tools (Weeks 9-10)
- Implement CLI development tools
- Add migration utilities
- Create benchmarking suite
- Write comprehensive documentation

### Phase 5: Integration & Polish (Weeks 11-12)
- End-to-end testing
- Performance optimization
- Security audit
- Release preparation

## Success Criteria

### Functional Requirements
- ✅ Drop-in compatibility with TypeScript MCP tools
- ✅ Support all existing configuration options
- ✅ Data migration from TypeScript version
- ✅ Same three-level compaction protocol
- ✅ PostgreSQL persistence with schema compatibility

### Performance Requirements
- ✅ 5x improvement in message storage throughput
- ✅ 5x improvement in compaction speed
- ✅ 2x reduction in memory usage
- ✅ 4x faster startup time
- ✅ 10x increase in concurrent session capacity

### Developer Experience Requirements
- ✅ Comprehensive test suite with property-based tests
- ✅ Rich CLI tooling for development
- ✅ Performance benchmarking suite
- ✅ Docker-first deployment model
- ✅ Detailed documentation and migration guide

## Risks and Mitigations

### Technical Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| MCP crate immaturity | High | Fork and maintain if needed, implement fallback |
| SQLx compile-time complexity | Medium | Start with runtime queries, migrate gradually |
| Performance regression | Medium | Continuous benchmarking against TypeScript version |
| Provider ecosystem limitations | Low | Custom HTTP client implementations as fallback |

### Project Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Scope creep | High | Strict adherence to this specification |
| Dependency on external crates | Medium | Evaluate alternatives, maintain compatibility layer |
| Migration complexity | Medium | Comprehensive testing, gradual rollout strategy |
| Documentation maintenance | Low | Automated documentation generation, regular reviews |

---

**Next Steps**: This specification provides the complete design for the Rust port. The next phase is to create a detailed implementation plan using the `writing-plans` skill, breaking down the work into actionable tasks with clear dependencies and timelines.