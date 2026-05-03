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

## Project Structure

```
src/
  types.ts        Core type definitions
  ids.ts          Type-safe ID factories
  store.ts        Immutable message store
  dag.ts          Summary DAG with lineage traversal
  compaction.ts   Three-level compaction engine
  context.ts      Active context window assembler
  retrieval.ts    lcm_describe / lcm_expand tools
  session.ts      Top-level session orchestrator
  defaults.ts     Default implementations & config presets
  index.ts        Public API barrel export
  lcm.test.ts     Test suite
raw/
  LCM.pdf         The original LCM paper
  volt-gh.md      Link to Volt source
```

## References

- [LCM: Lossless Context Management](https://papers.voltropy.com/LCM) — Clint Ehrlich, Voltropy PBC
- [Volt](https://github.com/Martian-Engineering/volt) — Coding agent with lossless context management
- [losslesscontext.ai](https://www.losslesscontext.ai/) — Visual explainer
