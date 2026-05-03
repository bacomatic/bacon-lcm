/**
 * OpenAI-compatible Summarizer
 *
 * Works with any endpoint that speaks the OpenAI Chat Completions API:
 *   - OpenAI (api.openai.com)
 *   - Azure OpenAI
 *   - Ollama (localhost:11434/v1)
 *   - LM Studio (localhost:1234/v1)
 *   - vLLM, TGI, etc.
 *
 * Uses native fetch — no SDK dependency required.
 */
import type { Summarizer, SummaryLevel } from "../types.js";
import type { SummarizerConfig } from "../config.js";

const DEFAULT_BASE_URL = "https://api.openai.com/v1";
const DEFAULT_MODEL = "gpt-4o-mini";

const LEVEL_INSTRUCTIONS: Record<SummaryLevel, string> = {
  leaf:
    "Create a concise summary of the following conversation messages. " +
    "Preserve all key facts, decisions, code references, file paths, and action items.",
  condensed:
    "Create a higher-level summary by merging the following summaries. " +
    "Eliminate redundancy while preserving all important details, decisions, and context.",
  emergency:
    "Create an extremely compressed summary of the following content. " +
    "Keep only the most critical facts, decisions, and action items. " +
    "This is an emergency compaction — brevity is paramount.",
};

export class OpenAISummarizer implements Summarizer {
  private readonly baseUrl: string;
  private readonly model: string;
  private readonly apiKey: string;
  private readonly maxTokens: number;
  private readonly temperature: number;
  private readonly systemPrompt: string;

  constructor(config: SummarizerConfig) {
    this.baseUrl = (config.baseUrl ?? DEFAULT_BASE_URL).replace(/\/+$/, "");
    this.model = config.model ?? DEFAULT_MODEL;
    this.apiKey = config.apiKey ?? "";
    this.maxTokens = config.maxTokens ?? 1024;
    this.temperature = config.temperature ?? 0.3;
    this.systemPrompt = config.systemPrompt ?? "";
  }

  async summarize(texts: string[], level: SummaryLevel): Promise<string> {
    const instruction = LEVEL_INSTRUCTIONS[level];
    const systemMsg = this.systemPrompt
      ? `${this.systemPrompt}\n\n${instruction}`
      : instruction;

    const numberedTexts = texts
      .map((t, i) => `[${i + 1}] ${t}`)
      .join("\n\n---\n\n");

    const headers: Record<string, string> = {
      "Content-Type": "application/json",
    };
    if (this.apiKey) {
      headers["Authorization"] = `Bearer ${this.apiKey}`;
    }

    const body = {
      model: this.model,
      messages: [
        { role: "system", content: systemMsg },
        { role: "user", content: numberedTexts },
      ],
      max_tokens: this.maxTokens,
      temperature: this.temperature,
    };

    const res = await fetch(`${this.baseUrl}/chat/completions`, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
    });

    if (!res.ok) {
      const errBody = await res.text().catch(() => "");
      throw new Error(
        `OpenAI-compatible API error (${res.status}): ${errBody.slice(0, 500)}`,
      );
    }

    const json = (await res.json()) as {
      choices: Array<{ message: { content: string } }>;
    };

    const content = json.choices?.[0]?.message?.content;
    if (!content) {
      throw new Error("OpenAI-compatible API returned no content");
    }

    return content.trim();
  }
}
