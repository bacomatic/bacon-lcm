/**
 * OpenAI-compatible Embedder
 *
 * Works with OpenAI, Azure OpenAI, Ollama, vLLM, LM Studio, and any other
 * endpoint that implements the /v1/embeddings API.
 */
import type { Embedder } from "../types.js";
import type { EmbedderConfig } from "../config.js";

const DEFAULT_BASE_URL = "https://api.openai.com/v1";
const DEFAULT_MODEL = "text-embedding-3-small";
const DEFAULT_DIMENSIONS = 1536;

export class OpenAIEmbedder implements Embedder {
  private readonly baseUrl: string;
  private readonly model: string;
  private readonly apiKey: string;
  readonly dimensions: number;

  constructor(config: EmbedderConfig) {
    this.baseUrl = (config.baseUrl ?? DEFAULT_BASE_URL).replace(/\/+$/, "");
    this.model = config.model ?? DEFAULT_MODEL;
    this.apiKey = config.apiKey ?? "";
    this.dimensions = config.dimensions ?? DEFAULT_DIMENSIONS;
  }

  async embed(text: string): Promise<number[]> {
    const results = await this.embedBatch([text]);
    return results[0];
  }

  async embedBatch(texts: string[]): Promise<number[][]> {
    const url = `${this.baseUrl}/embeddings`;
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
    };
    if (this.apiKey) {
      headers["Authorization"] = `Bearer ${this.apiKey}`;
    }

    const body: Record<string, unknown> = {
      input: texts,
      model: this.model,
    };

    // text-embedding-3-* supports explicit dimensions parameter
    if (this.model.startsWith("text-embedding-3")) {
      body.dimensions = this.dimensions;
    }

    const response = await fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
    });

    if (!response.ok) {
      const detail = await response.text();
      throw new Error(
        `OpenAI embeddings API error ${response.status}: ${detail}`,
      );
    }

    const json = (await response.json()) as {
      data: Array<{ embedding: number[]; index: number }>;
    };

    // Sort by index to preserve input order
    return json.data
      .sort((a, b) => a.index - b.index)
      .map((d) => d.embedding);
  }
}
