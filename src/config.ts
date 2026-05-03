/**
 * Configuration System
 *
 * Loads config from a JSON file with environment variable overrides.
 *
 * Resolution order:
 *   1. Explicit path via LCM_CONFIG env var
 *   2. ./bacon-lcm.config.json (cwd)
 *   3. ~/.config/bacon-lcm/config.json
 *   4. Built-in defaults
 *
 * Any value can be overridden by an environment variable.
 */
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import type { CompactionConfig, ThresholdConfig } from "./types.js";
import { DEFAULT_COMPACTION_CONFIG, DEFAULT_THRESHOLDS } from "./defaults.js";

// ---------------------------------------------------------------------------
// Config types
// ---------------------------------------------------------------------------

export interface TokenizerConfig {
  /** Type: "auto" | "tiktoken" | "anthropic" | "naive" */
  type: "auto" | "tiktoken" | "anthropic" | "naive";
  /** Model name for tiktoken encoding selection (e.g. "gpt-4o-mini") */
  model?: string;
  /** Explicit tiktoken encoding name (e.g. "o200k_base", "cl100k_base") */
  encoding?: string;
}

export interface SummarizerConfig {
  /** Provider: "openai" | "anthropic" | "echo" */
  provider: "openai" | "anthropic" | "echo";
  /** Model name (e.g. "gpt-4o-mini", "claude-sonnet-4-20250514") */
  model?: string;
  /** API key (prefer env var over file for secrets) */
  apiKey?: string;
  /** Base URL for OpenAI-compatible endpoints (OpenAI, Azure, Ollama, vLLM, LM Studio) */
  baseUrl?: string;
  /** Max tokens for the summary response */
  maxTokens?: number;
  /** Temperature (0–2) */
  temperature?: number;
  /** System prompt prepended to summarization requests */
  systemPrompt?: string;
}

export interface LcmConfig {
  /** Summarizer provider configuration */
  summarizer: SummarizerConfig;
  /** Token counter configuration */
  tokenizer?: TokenizerConfig;
  /** Compaction engine thresholds and parameters */
  compaction: CompactionConfig;
  /** PostgreSQL connection string */
  databaseUrl?: string;
  /** Dashboard settings */
  dashboard?: {
    enabled?: boolean;
    port?: number;
    host?: string;
  };
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

const DEFAULT_SUMMARIZER: SummarizerConfig = {
  provider: "echo",
  maxTokens: 1024,
  temperature: 0.3,
  systemPrompt:
    "You are a precise summarizer for an LLM context management system. " +
    "Produce concise summaries that preserve all key facts, decisions, " +
    "code references, and action items. Never invent information.",
};

const DEFAULT_CONFIG: LcmConfig = {
  summarizer: { ...DEFAULT_SUMMARIZER },
  compaction: { ...DEFAULT_COMPACTION_CONFIG },
};

// ---------------------------------------------------------------------------
// File loading
// ---------------------------------------------------------------------------

function findConfigFile(): string | null {
  const explicit = process.env.LCM_CONFIG;
  if (explicit) return resolve(explicit);

  const candidates = [
    join(process.cwd(), "bacon-lcm.config.json"),
    join(homedir(), ".config", "bacon-lcm", "config.json"),
  ];

  for (const p of candidates) {
    try {
      readFileSync(p, "utf-8");
      return p;
    } catch {
      // not found, try next
    }
  }
  return null;
}

function loadFileConfig(path: string): Partial<LcmConfig> {
  try {
    const raw = readFileSync(path, "utf-8");
    return JSON.parse(raw) as Partial<LcmConfig>;
  } catch (err) {
    console.error(`bacon-lcm: failed to load config from ${path}:`, err);
    return {};
  }
}

// ---------------------------------------------------------------------------
// Env overrides
// ---------------------------------------------------------------------------

function applyEnvOverrides(config: LcmConfig): void {
  // Summarizer
  if (process.env.LCM_SUMMARIZER_PROVIDER) {
    config.summarizer.provider = process.env.LCM_SUMMARIZER_PROVIDER as SummarizerConfig["provider"];
  }
  if (process.env.LCM_SUMMARIZER_MODEL) {
    config.summarizer.model = process.env.LCM_SUMMARIZER_MODEL;
  }
  if (process.env.LCM_SUMMARIZER_BASE_URL) {
    config.summarizer.baseUrl = process.env.LCM_SUMMARIZER_BASE_URL;
  }
  if (process.env.LCM_SUMMARIZER_MAX_TOKENS) {
    config.summarizer.maxTokens = parseInt(process.env.LCM_SUMMARIZER_MAX_TOKENS, 10);
  }
  if (process.env.LCM_SUMMARIZER_TEMPERATURE) {
    config.summarizer.temperature = parseFloat(process.env.LCM_SUMMARIZER_TEMPERATURE);
  }
  if (process.env.LCM_SUMMARIZER_SYSTEM_PROMPT) {
    config.summarizer.systemPrompt = process.env.LCM_SUMMARIZER_SYSTEM_PROMPT;
  }

  // API keys — check provider-specific env vars first, then generic
  if (process.env.OPENAI_API_KEY && !config.summarizer.apiKey) {
    config.summarizer.apiKey = process.env.OPENAI_API_KEY;
  }
  if (process.env.ANTHROPIC_API_KEY && !config.summarizer.apiKey) {
    config.summarizer.apiKey = process.env.ANTHROPIC_API_KEY;
  }
  if (process.env.LCM_API_KEY) {
    config.summarizer.apiKey = process.env.LCM_API_KEY;
  }

  // Database
  if (process.env.DATABASE_URL) {
    config.databaseUrl = process.env.DATABASE_URL;
  }

  // Dashboard
  if (process.env.DASHBOARD === "1") {
    config.dashboard = { ...config.dashboard, enabled: true };
  }
  if (process.env.DASHBOARD_PORT) {
    config.dashboard = {
      ...config.dashboard,
      enabled: true,
      port: parseInt(process.env.DASHBOARD_PORT, 10),
    };
  }

  // Tokenizer
  if (process.env.LCM_TOKENIZER) {
    config.tokenizer = {
      ...config.tokenizer,
      type: process.env.LCM_TOKENIZER as TokenizerConfig["type"],
    };
  }
  if (process.env.LCM_TOKENIZER_MODEL) {
    config.tokenizer = {
      ...config.tokenizer,
      type: config.tokenizer?.type ?? "tiktoken",
      model: process.env.LCM_TOKENIZER_MODEL,
    };
  }

  // Compaction thresholds
  if (process.env.LCM_MODEL_MAX_TOKENS) {
    config.compaction.thresholds.modelMaxTokens = parseInt(process.env.LCM_MODEL_MAX_TOKENS, 10);
  }
  if (process.env.LCM_SOFT_LIMIT) {
    config.compaction.thresholds.softLimit = parseInt(process.env.LCM_SOFT_LIMIT, 10);
  }
  if (process.env.LCM_HARD_LIMIT) {
    config.compaction.thresholds.hardLimit = parseInt(process.env.LCM_HARD_LIMIT, 10);
  }
  if (process.env.LCM_FRESH_TAIL_COUNT) {
    config.compaction.freshTailCount = parseInt(process.env.LCM_FRESH_TAIL_COUNT, 10);
  }
}

// ---------------------------------------------------------------------------
// Deep merge helper
// ---------------------------------------------------------------------------

function deepMerge<T extends Record<string, any>>(base: T, override: Partial<T>): T {
  const result = { ...base };
  for (const key of Object.keys(override) as (keyof T)[]) {
    const val = override[key];
    if (val !== undefined && val !== null) {
      if (typeof val === "object" && !Array.isArray(val) && typeof result[key] === "object") {
        result[key] = deepMerge(result[key] as any, val as any);
      } else {
        result[key] = val as T[keyof T];
      }
    }
  }
  return result;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

let cachedConfig: LcmConfig | null = null;

/**
 * Load the LCM configuration.
 * Merges: defaults ← config file ← env vars.
 * Result is cached; call `resetConfig()` to reload.
 */
export function loadConfig(): LcmConfig {
  if (cachedConfig) return cachedConfig;

  let config: LcmConfig = structuredClone(DEFAULT_CONFIG);

  const filePath = findConfigFile();
  if (filePath) {
    const fileConfig = loadFileConfig(filePath);
    config = deepMerge(config, fileConfig);
  }

  applyEnvOverrides(config);

  cachedConfig = config;
  return config;
}

/**
 * Clear the cached config, forcing a reload on next `loadConfig()` call.
 */
export function resetConfig(): void {
  cachedConfig = null;
}

/**
 * Get just the compaction config, with all overrides applied.
 */
export function getCompactionConfig(): CompactionConfig {
  return loadConfig().compaction;
}

/**
 * Get just the summarizer config, with all overrides applied.
 */
export function getSummarizerConfig(): SummarizerConfig {
  return loadConfig().summarizer;
}
