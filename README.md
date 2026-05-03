# bacon-lcm

**Lossless Context Memory** — a deterministic, database-backed context management system for LLM agents, modelled after the [Voltropy LCM paper](https://papers.voltropy.com/LCM) and [Volt](https://github.com/Martian-Engineering/volt).

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

| Level | Name        | Trigger                  | Mechanism                                      |
|-------|-------------|--------------------------|------------------------------------------------|
| 1     | Leaf        | Soft threshold exceeded  | Groups of raw messages → leaf summary nodes     |
| 2     | Condensed   | Still over after Level 1 | Groups of leaf nodes → condensed summary nodes  |
| 3     | Emergency   | Hard threshold exceeded  | Deterministic archival — no LLM call required   |

### Key Concepts

- **Fresh Tail** — the N most recent raw messages, always kept un-summarized for maximum fidelity
- **Lineage Pointers** — every summary node tracks which messages/nodes it was derived from, enabling lossless expansion
- **`lcm_describe`** — inspect a summary node's metadata (level, archived status, reachable message count)
- **`lcm_expand`** — follow lineage pointers to retrieve the original verbatim messages

## Quick Start

```bash
npm install
npm run build
npm test
```

## Usage

```typescript
import {
  LcmSession,
  NaiveTokenCounter,
  EchoSummarizer,
  DEFAULT_COMPACTION_CONFIG,
} from "bacon-lcm";

const session = new LcmSession(
  new NaiveTokenCounter(),
  new EchoSummarizer(),         // replace with your LLM-backed summarizer
  DEFAULT_COMPACTION_CONFIG,
);

// Add messages — compaction runs automatically when thresholds are exceeded
await session.addMessage("user", "Explain quantum computing");
await session.addMessage("assistant", "Quantum computing uses qubits...");

// Get the active context window (summaries + fresh tail)
const context = session.getContext();

// Inspect a summary
const summaries = context.filter(item => item.kind === "summary");
if (summaries.length > 0) {
  const desc = session.describe(summaries[0].summary.id);
  const original = session.expand(summaries[0].summary.id);
}
```

## Pluggable Interfaces

| Interface      | Purpose                          | Default                    |
|----------------|----------------------------------|----------------------------|
| `TokenCounter` | Estimate token count for a text  | `NaiveTokenCounter` (~4 c/t) |
| `Summarizer`   | LLM call to produce summaries    | `EchoSummarizer` (testing) |
| `MessageStore` | Persistence for raw messages     | `InMemoryMessageStore`     |
| `SummaryDag`   | Persistence for the summary DAG  | `InMemorySummaryDag`       |

Replace `NaiveTokenCounter` with tiktoken or your model's tokenizer, and `EchoSummarizer` with an actual LLM call for production use.

## PostgreSQL Persistence

For durable, cross-session memory, swap the in-memory stores for Postgres-backed ones:

```bash
# 1. Create the database and run the migration
createdb bacon_lcm
psql bacon_lcm < sql/001_init.sql
```

```typescript
import pg from "pg";
import {
  LcmSession,
  NaiveTokenCounter,
  EchoSummarizer,
  PgMessageStore,
  PgSummaryDag,
  DEFAULT_COMPACTION_CONFIG,
} from "bacon-lcm";

const pool = new pg.Pool({ connectionString: "postgres://localhost:5432/bacon_lcm" });
const tokenCounter = new NaiveTokenCounter();

const store = new PgMessageStore(pool, tokenCounter);
const dag = new PgSummaryDag(pool, tokenCounter);

// Optionally auto-migrate (safe to call repeatedly)
await store.migrate();
await dag.migrate();

const session = new LcmSession(
  tokenCounter,
  new EchoSummarizer(),
  DEFAULT_COMPACTION_CONFIG,
  { store, dag },   // <-- inject Postgres-backed stores
);

await session.addMessage("user", "This will be persisted to Postgres");
```

All `MessageStore` and `SummaryDag` interface methods are natively async (`Promise`-returning). The in-memory and Postgres implementations share the same interface, so switching between them requires no code changes beyond the constructor.

### MCP Server with Postgres

Set `DATABASE_URL` when starting the MCP server for persistent cross-session memory:

```json
{
  "mcpServers": {
    "bacon-lcm": {
      "command": "node",
      "args": ["/path/to/bacon-lcm/dist/mcp-server.js"],
      "env": { "DATABASE_URL": "postgres://localhost:5432/bacon_lcm" }
    }
  }
}
```

Without `DATABASE_URL`, the MCP server falls back to in-memory storage.

## Integration: MCP Server

The MCP server exposes LCM as tools that any MCP-compatible agent can call.

### Windsurf

Add to `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "bacon-lcm": {
      "command": "node",
      "args": ["/path/to/bacon-lcm/dist/mcp-server.js"]
    }
  }
}
```

### Devin

Add the same MCP server via Devin's MCP Marketplace or config.

### Copilot CLI / Any MCP Host

Any agent that speaks MCP over stdio can connect to `dist/mcp-server.js`.

### MCP Tools

| Tool | Description |
|------|-------------|
| `lcm_store` | Persist a message; auto-compaction when thresholds exceeded |
| `lcm_recall` | Retrieve active context window (summaries + fresh tail) |
| `lcm_describe` | Inspect a summary node's lineage metadata |
| `lcm_expand` | Expand a summary to original verbatim messages |
| `lcm_session_new` | Create a new LCM session |
| `lcm_session_info` | Get current session statistics |

## Integration: Hooks (Passive Capture)

Hooks silently capture every prompt and response into the LCM store, without the agent needing to call tools. Build first: `npm run build`.

### Windsurf Cascade Hooks

Copy or symlink `.windsurf/hooks.json` (included in repo) to your project. It captures:
- `pre_user_prompt` — every user message
- `post_cascade_response` — every assistant response
- `post_cascade_response_with_transcript` — full session transcripts

### GitHub Copilot CLI Hooks

Copy `.github/hooks/lcm.json` to your repo's `.github/hooks/` directory. It captures:
- `sessionStart` / `sessionEnd` — session lifecycle
- `userPromptSubmitted` — every user prompt
- `preToolUse` / `postToolUse` — tool invocations

### Hook CLI

Both hook configs call the same unified CLI:

```bash
# Windsurf (auto-detects platform from JSON shape)
echo '{"agent_action_name":"pre_user_prompt","tool_info":{"user_prompt":"hello"}}' | node dist/hooks/cli.js

# Copilot CLI (requires --hook flag)
echo '{"timestamp":123,"cwd":".","prompt":"hello"}' | node dist/hooks/cli.js --platform copilot --hook userPromptSubmitted
```

## Project Structure

```
src/
  types.ts          Core type definitions
  ids.ts            Type-safe ID factories
  store.ts          Immutable message store
  dag.ts            Summary DAG with lineage traversal
  compaction.ts     Three-level compaction engine
  context.ts        Active context window assembler
  retrieval.ts      lcm_describe / lcm_expand tools
  session.ts        Top-level session orchestrator
  defaults.ts       Default implementations & config presets
  mcp-server.ts     MCP server (stdio transport)
  index.ts          Public API barrel export
  lcm.test.ts       Core test suite
  hooks/
    handler.ts      Unified hook handler (platform-agnostic)
    windsurf.ts     Windsurf Cascade hooks adapter
    copilot.ts      Copilot CLI hooks adapter
    cli.ts          CLI entry point for hook scripts
    index.ts        Hooks barrel export
    hooks.test.ts   Hook test suite
  pg/
    pg-store.ts     PostgreSQL message store
    pg-dag.ts       PostgreSQL summary DAG
    index.ts        Pg barrel export
    pg.test.ts      Postgres integration tests
sql/
  001_init.sql      Database migration
.windsurf/
  hooks.json        Windsurf hooks config (ready to use)
.github/
  hooks/
    lcm.json        Copilot CLI hooks config (ready to use)
raw/
  LCM.pdf           The original LCM paper
  volt-gh.md        Link to Volt source
```

## References

- [LCM: Lossless Context Management](https://papers.voltropy.com/LCM) — Clint Ehrlich, Voltropy PBC
- [Volt](https://github.com/Martian-Engineering/volt) — Coding agent with lossless context management
- [losslesscontext.ai](https://www.losslesscontext.ai/) — Visual explainer
