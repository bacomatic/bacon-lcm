/**
 * Token Counter Factory
 *
 * Creates the appropriate TokenCounter based on config.
 * Auto-selects based on the summarizer provider when tokenizer is set to "auto":
 *   - openai  → TiktokenCounter (model-aware)
 *   - anthropic → AnthropicTokenCounter (calibrated heuristic)
 *   - echo/other → NaiveTokenCounter (simple ~4 c/t heuristic)
 */
import type { TokenCounter } from "../types.js";
import type { LcmConfig, TokenizerConfig } from "../config.js";
import { NaiveTokenCounter } from "../defaults.js";
import { TiktokenCounter, type TiktokenEncoding } from "./tiktoken.js";
import { AnthropicTokenCounter } from "./anthropic.js";

/**
 * Create a TokenCounter from the given config.
 * When tokenizer.type is "auto" (or unset), the counter is selected based on
 * the summarizer provider and model.
 */
export function createTokenCounter(config: LcmConfig): TokenCounter {
  const tc = config.tokenizer ?? { type: "auto" };

  switch (tc.type) {
    case "tiktoken":
      return new TiktokenCounter({
        model: tc.model ?? config.summarizer.model,
        encoding: tc.encoding as TiktokenEncoding | undefined,
      });

    case "anthropic":
      return new AnthropicTokenCounter();

    case "naive":
      return new NaiveTokenCounter();

    case "auto":
    default:
      return autoSelect(config);
  }
}

function autoSelect(config: LcmConfig): TokenCounter {
  switch (config.summarizer.provider) {
    case "openai":
      return new TiktokenCounter({ model: config.summarizer.model });
    case "anthropic":
      return new AnthropicTokenCounter();
    case "echo":
    default:
      return new NaiveTokenCounter();
  }
}

export { TiktokenCounter } from "./tiktoken.js";
export type { TiktokenCounterOptions, TiktokenEncoding } from "./tiktoken.js";
export { AnthropicTokenCounter } from "./anthropic.js";
