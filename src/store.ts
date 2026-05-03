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
  ): Message;

  /** Retrieve a single message by ID. */
  get(id: MessageId): Message | undefined;

  /** Retrieve all messages for a session, ordered by sequence number. */
  getBySession(sessionId: SessionId): Message[];

  /** Retrieve a contiguous range of messages by sequence number (inclusive). */
  getRange(sessionId: SessionId, from: number, to: number): Message[];

  /** Retrieve specific messages by their IDs, preserving order. */
  getMany(ids: MessageId[]): Message[];

  /** Total number of messages stored. */
  size(): number;
}

// ---------------------------------------------------------------------------
// In-memory implementation
// ---------------------------------------------------------------------------

export class InMemoryMessageStore implements MessageStore {
  private readonly messages = new Map<MessageId, Message>();
  private readonly sessionIndex = new Map<SessionId, Message[]>();
  private readonly sessionSeq = new Map<SessionId, number>();

  constructor(private readonly tokenCounter: TokenCounter) {}

  append(
    sessionId: SessionId,
    role: MessageRole,
    content: string,
    metadata?: Record<string, unknown>,
  ): Message {
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

  get(id: MessageId): Message | undefined {
    return this.messages.get(id);
  }

  getBySession(sessionId: SessionId): Message[] {
    return this.sessionIndex.get(sessionId) ?? [];
  }

  getRange(sessionId: SessionId, from: number, to: number): Message[] {
    return this.getBySession(sessionId).filter(
      (m) => m.sequenceNumber >= from && m.sequenceNumber <= to,
    );
  }

  getMany(ids: MessageId[]): Message[] {
    const result: Message[] = [];
    for (const id of ids) {
      const m = this.messages.get(id);
      if (m) result.push(m);
    }
    return result;
  }

  size(): number {
    return this.messages.size;
  }
}
