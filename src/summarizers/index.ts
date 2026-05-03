/**
 * Summarizer Factory
 *
 * Creates the appropriate Summarizer implementation based on config.
 */
import type { Summarizer } from "../types.js";
import type { SummarizerConfig } from "../config.js";
import { EchoSummarizer } from "../defaults.js";
import { OpenAISummarizer } from "./openai.js";
import { AnthropicSummarizer } from "./anthropic.js";

/**
 * Create a Summarizer from the given config.
 * Falls back to EchoSummarizer if the provider is "echo" or unrecognized.
 */
export function createSummarizer(config: SummarizerConfig): Summarizer {
  switch (config.provider) {
    case "openai":
      return new OpenAISummarizer(config);
    case "anthropic":
      return new AnthropicSummarizer(config);
    case "echo":
    default:
      return new EchoSummarizer();
  }
}

export { OpenAISummarizer } from "./openai.js";
export { AnthropicSummarizer } from "./anthropic.js";
