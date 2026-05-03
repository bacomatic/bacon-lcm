/**
 * Anthropic Token Counter
 *
 * Heuristic-based token estimation calibrated to Claude's tokenizer.
 * Claude uses a custom BPE tokenizer that averages ~3.4 characters per token
 * for English text (slightly more efficient than OpenAI's cl100k_base at ~4 c/t).
 *
 * This provides a good estimate without requiring Anthropic's proprietary
 * tokenizer. For exact counts, use the Anthropic API's token counting endpoint.
 */
import type { TokenCounter } from "../types.js";

const CHARS_PER_TOKEN = 3.4;

export class AnthropicTokenCounter implements TokenCounter {
  count(text: string): number {
    return Math.ceil(text.length / CHARS_PER_TOKEN);
  }
}
