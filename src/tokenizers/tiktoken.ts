/**
 * Tiktoken-based Token Counter
 *
 * Accurate token counting for OpenAI models using the js-tiktoken library.
 * Supports model-based encoding selection (o200k_base for GPT-4o family,
 * cl100k_base for GPT-4/3.5) or explicit encoding name.
 */
import { encodingForModel, getEncoding, type Tiktoken } from "js-tiktoken";
import type { TokenCounter } from "../types.js";

/** Known encoding names that js-tiktoken supports. */
export type TiktokenEncoding = "o200k_base" | "cl100k_base" | "p50k_base" | "r50k_base" | "gpt2";

export interface TiktokenCounterOptions {
  /** OpenAI model name — encoding is auto-detected. Takes precedence over `encoding`. */
  model?: string;
  /** Explicit encoding name. Used when model is not set. Default: "o200k_base". */
  encoding?: TiktokenEncoding;
}

export class TiktokenCounter implements TokenCounter {
  private readonly enc: Tiktoken;

  constructor(opts?: TiktokenCounterOptions) {
    if (opts?.model) {
      try {
        this.enc = encodingForModel(opts.model as Parameters<typeof encodingForModel>[0]);
      } catch {
        // Unknown model — fall back to the latest encoding
        this.enc = getEncoding(opts?.encoding ?? "o200k_base");
      }
    } else {
      this.enc = getEncoding(opts?.encoding ?? "o200k_base");
    }
  }

  count(text: string): number {
    return this.enc.encode(text).length;
  }
}
