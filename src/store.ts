/**
 * Immutable Message Store
 *
 * The source of truth for every message produced during a session.
 * Messages are append-only — once written they are never modified or deleted.
 *
 * This in-memory implementation can be swapped for a Postgres-backed store
 * without changing any consumer code (see the `MessageStore` interface).
 */
import type {
  Message,
  MessageId,
  MessageRole,
  SessionId,
  TokenCounter,
} from "./types.js";
import { newMessageId } from "./ids.js";

// ---------------------------------------------------------------------------
// Interface
// ---------------------------------------------------------------------------

export interface MessageStore {
  /** Append a new message and return its persisted form. */
  append(
    sessionId: SessionId,
    role: MessageRole,
    content: string,
    metadata?: Record<string, unknown>,
  ): Promise<Message>;

  /** Retrieve a single message by ID. */
  get(id: MessageId): Promise<Message | undefined>;

  /** Retrieve all messages for a session, ordered by sequence number. */
  getBySession(sessionId: SessionId): Promise<Message[]>;

  /** Retrieve a contiguous range of messages by sequence number (inclusive). */
  getRange(sessionId: SessionId, from: number, to: number): Promise<Message[]>;

  /** Retrieve specific messages by their IDs, preserving order. */
  getMany(ids: MessageId[]): Promise<Message[]>;

  /** Total number of messages stored. */
  size(): Promise<number>;
}

// ---------------------------------------------------------------------------
// In-memory implementation
// ---------------------------------------------------------------------------

export class InMemoryMessageStore implements MessageStore {
  private readonly messages = new Map<MessageId, Message>();
  private readonly sessionIndex = new Map<SessionId, Message[]>();
  private readonly sessionSeq = new Map<SessionId, number>();

  constructor(private readonly tokenCounter: TokenCounter) {}

  async append(
    sessionId: SessionId,
    role: MessageRole,
    content: string,
    metadata?: Record<string, unknown>,
  ): Promise<Message> {
    const seq = (this.sessionSeq.get(sessionId) ?? 0) + 1;
    this.sessionSeq.set(sessionId, seq);

    const message: Message = {
      id: newMessageId(),
      sessionId,
      role,
      content,
      sequenceNumber: seq,
      tokenCount: this.tokenCounter.count(content),
      createdAt: new Date(),
      metadata,
    };

    this.messages.set(message.id, message);

    let list = this.sessionIndex.get(sessionId);
    if (!list) {
      list = [];
      this.sessionIndex.set(sessionId, list);
    }
    list.push(message);

    return message;
  }

  async get(id: MessageId): Promise<Message | undefined> {
    return this.messages.get(id);
  }

  async getBySession(sessionId: SessionId): Promise<Message[]> {
    return this.sessionIndex.get(sessionId) ?? [];
  }

  async getRange(sessionId: SessionId, from: number, to: number): Promise<Message[]> {
    return (await this.getBySession(sessionId)).filter(
      (m) => m.sequenceNumber >= from && m.sequenceNumber <= to,
    );
  }

  async getMany(ids: MessageId[]): Promise<Message[]> {
    const result: Message[] = [];
    for (const id of ids) {
      const m = this.messages.get(id);
      if (m) result.push(m);
    }
    return result;
  }

  async size(): Promise<number> {
    return this.messages.size;
  }
}
