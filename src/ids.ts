/**
 * Type-safe ID factory functions.
 */
import { v4 as uuidv4 } from "uuid";
import type { MessageId, SessionId, SummaryId } from "./types.js";

export function newMessageId(): MessageId {
  return uuidv4() as MessageId;
}

export function newSummaryId(): SummaryId {
  return uuidv4() as SummaryId;
}

export function newSessionId(): SessionId {
  return uuidv4() as SessionId;
}
