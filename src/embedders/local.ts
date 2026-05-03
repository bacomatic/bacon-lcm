/**
 * Local Embedder via @huggingface/transformers
 *
 * Runs a small embedding model locally — no API key needed.
 * Default model: Xenova/all-MiniLM-L6-v2 (384 dimensions, ~23MB).
 *
 * The @huggingface/transformers package is an optional peer dependency.
 * It will be lazily imported only when this embedder is instantiated.
 */
import type { Embedder } from "../types.js";
import type { EmbedderConfig } from "../config.js";

const DEFAULT_MODEL = "Xenova/all-MiniLM-L6-v2";
const DEFAULT_DIMENSIONS = 384;

export class LocalEmbedder implements Embedder {
  private readonly model: string;
  readonly dimensions: number;
  private pipeline: any = null;

  constructor(config: EmbedderConfig) {
    this.model = config.model ?? DEFAULT_MODEL;
    this.dimensions = config.dimensions ?? DEFAULT_DIMENSIONS;
  }

  private async getPipeline(): Promise<any> {
    if (this.pipeline) return this.pipeline;

    let transformers: any;
    try {
      // @ts-ignore -- optional peer dependency, dynamically imported
      transformers = await import("@huggingface/transformers");
    } catch {
      throw new Error(
        "LocalEmbedder requires @huggingface/transformers. " +
          "Install it with: npm install @huggingface/transformers",
      );
    }

    this.pipeline = await transformers.pipeline(
      "feature-extraction",
      this.model,
      { dtype: "fp32" },
    );
    return this.pipeline;
  }

  async embed(text: string): Promise<number[]> {
    const results = await this.embedBatch([text]);
    return results[0];
  }

  async embedBatch(texts: string[]): Promise<number[][]> {
    const pipe = await this.getPipeline();
    const results: number[][] = [];

    for (const text of texts) {
      const output = await pipe(text, { pooling: "mean", normalize: true });
      // output is a Tensor — convert to flat array
      const arr = Array.from(output.data as Float32Array) as number[];
      // The output may be a flattened [1, dimensions] tensor
      results.push(arr.slice(0, this.dimensions));
    }

    return results;
  }
}
