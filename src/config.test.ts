import { afterEach, describe, expect, it, vi } from "vitest";
import { loadConfig, resetConfig } from "./config.js";
import { createSummarizer, OpenAISummarizer, AnthropicSummarizer } from "./summarizers/index.js";
import { createTokenCounter, TiktokenCounter, AnthropicTokenCounter } from "./tokenizers/index.js";
import { EchoSummarizer, NaiveTokenCounter } from "./defaults.js";

describe("loadConfig", () => {
  afterEach(() => {
    resetConfig();
    // Clean up env vars we may have set
    delete process.env.LCM_SUMMARIZER_PROVIDER;
    delete process.env.LCM_SUMMARIZER_MODEL;
    delete process.env.LCM_SUMMARIZER_BASE_URL;
    delete process.env.LCM_SUMMARIZER_MAX_TOKENS;
    delete process.env.LCM_SUMMARIZER_TEMPERATURE;
    delete process.env.LCM_API_KEY;
    delete process.env.OPENAI_API_KEY;
    delete process.env.ANTHROPIC_API_KEY;
    delete process.env.DATABASE_URL;
    delete process.env.DASHBOARD;
    delete process.env.DASHBOARD_PORT;
    delete process.env.LCM_CONFIG;
    delete process.env.LCM_MODEL_MAX_TOKENS;
    delete process.env.LCM_SOFT_LIMIT;
    delete process.env.LCM_HARD_LIMIT;
    delete process.env.LCM_FRESH_TAIL_COUNT;
    delete process.env.LCM_TOKENIZER;
    delete process.env.LCM_TOKENIZER_MODEL;
  });

  it("returns defaults when no config file or env vars", () => {
    // Ensure no env vars leak from the test runner
    delete process.env.DATABASE_URL;
    resetConfig();
    const cfg = loadConfig();
    expect(cfg.summarizer.provider).toBe("echo");
    expect(cfg.compaction.thresholds.modelMaxTokens).toBe(128_000);
    expect(cfg.databaseUrl).toBeUndefined();
    expect(cfg.dashboard).toBeUndefined();
  });

  it("caches config and returns same object", () => {
    const a = loadConfig();
    const b = loadConfig();
    expect(a).toBe(b);
  });

  it("resets cache", () => {
    const a = loadConfig();
    resetConfig();
    const b = loadConfig();
    expect(a).not.toBe(b);
    expect(a).toEqual(b);
  });

  it("applies LCM_SUMMARIZER_PROVIDER env override", () => {
    process.env.LCM_SUMMARIZER_PROVIDER = "openai";
    const cfg = loadConfig();
    expect(cfg.summarizer.provider).toBe("openai");
  });

  it("applies LCM_SUMMARIZER_MODEL env override", () => {
    process.env.LCM_SUMMARIZER_MODEL = "gpt-4o";
    const cfg = loadConfig();
    expect(cfg.summarizer.model).toBe("gpt-4o");
  });

  it("applies LCM_SUMMARIZER_BASE_URL env override", () => {
    process.env.LCM_SUMMARIZER_BASE_URL = "http://localhost:11434/v1";
    const cfg = loadConfig();
    expect(cfg.summarizer.baseUrl).toBe("http://localhost:11434/v1");
  });

  it("applies LCM_API_KEY env override (highest priority)", () => {
    process.env.OPENAI_API_KEY = "sk-openai";
    process.env.LCM_API_KEY = "sk-override";
    const cfg = loadConfig();
    expect(cfg.summarizer.apiKey).toBe("sk-override");
  });

  it("applies OPENAI_API_KEY when no LCM_API_KEY", () => {
    process.env.OPENAI_API_KEY = "sk-openai";
    const cfg = loadConfig();
    expect(cfg.summarizer.apiKey).toBe("sk-openai");
  });

  it("applies DATABASE_URL env override", () => {
    process.env.DATABASE_URL = "postgres://localhost:5432/test_db";
    const cfg = loadConfig();
    expect(cfg.databaseUrl).toBe("postgres://localhost:5432/test_db");
  });

  it("applies DASHBOARD=1 env override", () => {
    process.env.DASHBOARD = "1";
    const cfg = loadConfig();
    expect(cfg.dashboard?.enabled).toBe(true);
  });

  it("applies DASHBOARD_PORT env override", () => {
    process.env.DASHBOARD_PORT = "4000";
    const cfg = loadConfig();
    expect(cfg.dashboard?.enabled).toBe(true);
    expect(cfg.dashboard?.port).toBe(4000);
  });

  it("applies compaction threshold overrides", () => {
    process.env.LCM_MODEL_MAX_TOKENS = "200000";
    process.env.LCM_SOFT_LIMIT = "100000";
    process.env.LCM_HARD_LIMIT = "150000";
    process.env.LCM_FRESH_TAIL_COUNT = "20";
    const cfg = loadConfig();
    expect(cfg.compaction.thresholds.modelMaxTokens).toBe(200000);
    expect(cfg.compaction.thresholds.softLimit).toBe(100000);
    expect(cfg.compaction.thresholds.hardLimit).toBe(150000);
    expect(cfg.compaction.freshTailCount).toBe(20);
  });

  it("applies LCM_TOKENIZER env override", () => {
    process.env.LCM_TOKENIZER = "tiktoken";
    const cfg = loadConfig();
    expect(cfg.tokenizer?.type).toBe("tiktoken");
  });

  it("applies LCM_TOKENIZER_MODEL env override", () => {
    process.env.LCM_TOKENIZER_MODEL = "gpt-4o";
    const cfg = loadConfig();
    expect(cfg.tokenizer?.type).toBe("tiktoken");
    expect(cfg.tokenizer?.model).toBe("gpt-4o");
  });
});

describe("createSummarizer", () => {
  it("creates EchoSummarizer for 'echo' provider", () => {
    const s = createSummarizer({ provider: "echo" });
    expect(s).toBeInstanceOf(EchoSummarizer);
  });

  it("creates OpenAISummarizer for 'openai' provider", () => {
    const s = createSummarizer({ provider: "openai", model: "gpt-4o-mini" });
    expect(s).toBeInstanceOf(OpenAISummarizer);
  });

  it("creates AnthropicSummarizer for 'anthropic' provider", () => {
    const s = createSummarizer({ provider: "anthropic", apiKey: "sk-test" });
    expect(s).toBeInstanceOf(AnthropicSummarizer);
  });

  it("falls back to EchoSummarizer for unknown provider", () => {
    const s = createSummarizer({ provider: "unknown" as any });
    expect(s).toBeInstanceOf(EchoSummarizer);
  });
});

describe("OpenAISummarizer", () => {
  it("constructs with defaults", () => {
    const s = new OpenAISummarizer({ provider: "openai" });
    expect(s).toBeInstanceOf(OpenAISummarizer);
  });

  it("calls the API and parses the response", async () => {
    const mockResponse = {
      choices: [{ message: { content: "Test summary" } }],
    };

    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      new Response(JSON.stringify(mockResponse), { status: 200 }),
    );

    const s = new OpenAISummarizer({
      provider: "openai",
      apiKey: "sk-test",
      baseUrl: "https://api.example.com/v1",
      model: "test-model",
    });

    const result = await s.summarize(["Hello", "World"], "leaf");
    expect(result).toBe("Test summary");
    expect(fetchSpy).toHaveBeenCalledOnce();

    const [url, opts] = fetchSpy.mock.calls[0];
    expect(url).toBe("https://api.example.com/v1/chat/completions");
    expect((opts?.headers as Record<string, string>)["Authorization"]).toBe("Bearer sk-test");

    const body = JSON.parse(opts?.body as string);
    expect(body.model).toBe("test-model");
    expect(body.messages).toHaveLength(2);

    fetchSpy.mockRestore();
  });

  it("throws on API error", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      new Response("Rate limited", { status: 429 }),
    );

    const s = new OpenAISummarizer({ provider: "openai" });
    await expect(s.summarize(["text"], "leaf")).rejects.toThrow("429");

    fetchSpy.mockRestore();
  });
});

describe("AnthropicSummarizer", () => {
  it("constructs with defaults", () => {
    const s = new AnthropicSummarizer({ provider: "anthropic" });
    expect(s).toBeInstanceOf(AnthropicSummarizer);
  });

  it("calls the API and parses the response", async () => {
    const mockResponse = {
      content: [{ type: "text", text: "Anthropic summary" }],
    };

    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      new Response(JSON.stringify(mockResponse), { status: 200 }),
    );

    const s = new AnthropicSummarizer({
      provider: "anthropic",
      apiKey: "sk-ant-test",
      model: "claude-sonnet-4-20250514",
    });

    const result = await s.summarize(["Hello", "World"], "condensed");
    expect(result).toBe("Anthropic summary");
    expect(fetchSpy).toHaveBeenCalledOnce();

    const [url, opts] = fetchSpy.mock.calls[0];
    expect(url).toBe("https://api.anthropic.com/v1/messages");
    expect((opts?.headers as Record<string, string>)["x-api-key"]).toBe("sk-ant-test");
    expect((opts?.headers as Record<string, string>)["anthropic-version"]).toBe("2023-06-01");

    const body = JSON.parse(opts?.body as string);
    expect(body.model).toBe("claude-sonnet-4-20250514");
    expect(body.system).toContain("higher-level summary");

    fetchSpy.mockRestore();
  });

  it("throws on API error", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      new Response("Unauthorized", { status: 401 }),
    );

    const s = new AnthropicSummarizer({ provider: "anthropic" });
    await expect(s.summarize(["text"], "leaf")).rejects.toThrow("401");

    fetchSpy.mockRestore();
  });
});

// ---------------------------------------------------------------------------
// Token Counters
// ---------------------------------------------------------------------------

describe("createTokenCounter", () => {
  it("auto-selects TiktokenCounter for openai provider", () => {
    const tc = createTokenCounter({
      summarizer: { provider: "openai", model: "gpt-4o-mini" },
      compaction: {} as any,
    });
    expect(tc).toBeInstanceOf(TiktokenCounter);
  });

  it("auto-selects AnthropicTokenCounter for anthropic provider", () => {
    const tc = createTokenCounter({
      summarizer: { provider: "anthropic" },
      compaction: {} as any,
    });
    expect(tc).toBeInstanceOf(AnthropicTokenCounter);
  });

  it("auto-selects NaiveTokenCounter for echo provider", () => {
    const tc = createTokenCounter({
      summarizer: { provider: "echo" },
      compaction: {} as any,
    });
    expect(tc).toBeInstanceOf(NaiveTokenCounter);
  });

  it("respects explicit tiktoken type", () => {
    const tc = createTokenCounter({
      summarizer: { provider: "echo" },
      tokenizer: { type: "tiktoken" },
      compaction: {} as any,
    });
    expect(tc).toBeInstanceOf(TiktokenCounter);
  });

  it("respects explicit anthropic type", () => {
    const tc = createTokenCounter({
      summarizer: { provider: "openai" },
      tokenizer: { type: "anthropic" },
      compaction: {} as any,
    });
    expect(tc).toBeInstanceOf(AnthropicTokenCounter);
  });

  it("respects explicit naive type", () => {
    const tc = createTokenCounter({
      summarizer: { provider: "openai" },
      tokenizer: { type: "naive" },
      compaction: {} as any,
    });
    expect(tc).toBeInstanceOf(NaiveTokenCounter);
  });
});

describe("TiktokenCounter", () => {
  it("counts tokens accurately for known text", () => {
    const tc = new TiktokenCounter({ model: "gpt-4o-mini" });
    // "Hello world" = 2 tokens with o200k_base
    expect(tc.count("Hello world")).toBe(2);
  });

  it("handles empty string", () => {
    const tc = new TiktokenCounter();
    expect(tc.count("")).toBe(0);
  });

  it("handles multi-line code", () => {
    const tc = new TiktokenCounter();
    const code = 'function hello() {\n  return "world";\n}';
    const tokens = tc.count(code);
    expect(tokens).toBeGreaterThan(0);
    expect(tokens).toBeLessThan(20);
  });

  it("falls back to o200k_base for unknown model", () => {
    const tc = new TiktokenCounter({ model: "nonexistent-model-xyz" });
    expect(tc.count("Hello")).toBeGreaterThan(0);
  });

  it("accepts explicit encoding", () => {
    const tc = new TiktokenCounter({ encoding: "cl100k_base" });
    expect(tc.count("Hello world")).toBeGreaterThan(0);
  });
});

describe("AnthropicTokenCounter", () => {
  it("estimates tokens with ~3.4 chars/token ratio", () => {
    const tc = new AnthropicTokenCounter();
    // 34 chars / 3.4 = 10 tokens
    const text = "a".repeat(34);
    expect(tc.count(text)).toBe(10);
  });

  it("handles empty string", () => {
    const tc = new AnthropicTokenCounter();
    expect(tc.count("")).toBe(0);
  });

  it("is more aggressive than NaiveTokenCounter", () => {
    const anthropic = new AnthropicTokenCounter();
    const naive = new NaiveTokenCounter();
    const text = "This is a sample sentence for comparison.";
    // AnthropicTokenCounter (~3.4 c/t) should give higher count than NaiveTokenCounter (~4 c/t)
    expect(anthropic.count(text)).toBeGreaterThan(naive.count(text));
  });
});
