/**
 * Embedder Factory
 *
 * Creates the appropriate Embedder based on config.
 * Auto-selects based on the summarizer provider when embedder is "none" or unset:
 *   - openai provider → OpenAIEmbedder (uses summarizer's API key/baseUrl as fallback)
 *   - other → NullEmbedder (no-op, semantic search disabled)
 */
import type { Embedder } from "../types.js";
import type { LcmConfig, EmbedderConfig } from "../config.js";
import { OpenAIEmbedder } from "./openai.js";
import { LocalEmbedder } from "./local.js";

/**
 * A no-op embedder that disables semantic search.
 */
export class NullEmbedder implements Embedder {
  readonly dimensions = 0;

  async embed(_text: string): Promise<number[]> {
    return [];
  }
  async embedBatch(texts: string[]): Promise<number[][]> {
    return texts.map(() => []);
  }
}

/**
 * Create an Embedder from the given config.
 * Falls back to NullEmbedder when no embedder is configured.
 */
export function createEmbedder(config: LcmConfig): Embedder {
  const ec = config.embedder;
  if (!ec || ec.provider === "none") {
    return new NullEmbedder();
  }

  switch (ec.provider) {
    case "openai": {
      // Inherit API key / base URL from summarizer if not set
      const resolved: EmbedderConfig = {
        ...ec,
        apiKey: ec.apiKey ?? config.summarizer.apiKey,
        baseUrl: ec.baseUrl ?? config.summarizer.baseUrl,
      };
      return new OpenAIEmbedder(resolved);
    }

    case "local":
      return new LocalEmbedder(ec);

    default:
      return new NullEmbedder();
  }
}

export { OpenAIEmbedder } from "./openai.js";
export { LocalEmbedder } from "./local.js";
