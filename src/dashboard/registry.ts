/**
 * Shared LCM Session Registry
 *
 * Provides a central registry of LcmSession instances that can be
 * observed by the dashboard and manipulated by the MCP server or hooks.
 */
import type { LcmSession } from "../session.js";

export interface SessionSnapshot {
  sessionId: string;
  createdAt: string;
  activeTokenCount: number;
  totalMessages: number;
  activeSummaries: number;
  archivedSummaries: number;
  totalSummaryNodes: number;
  contextItems: number;
  isActive: boolean;
}

export interface DashboardOverview {
  timestamp: string;
  sessionCount: number;
  activeSessionId: string | null;
  sessions: SessionSnapshot[];
}

class SessionRegistry {
  private readonly sessions = new Map<string, LcmSession>();
  private activeId: string | null = null;

  register(session: LcmSession): void {
    this.sessions.set(session.session.id, session);
  }

  unregister(sessionId: string): void {
    this.sessions.delete(sessionId);
    if (this.activeId === sessionId) this.activeId = null;
  }

  setActive(sessionId: string): void {
    this.activeId = sessionId;
  }

  getActive(): LcmSession | undefined {
    if (!this.activeId) return undefined;
    return this.sessions.get(this.activeId);
  }

  get(sessionId: string): LcmSession | undefined {
    return this.sessions.get(sessionId);
  }

  get activeSessionId(): string | null {
    return this.activeId;
  }

  get all(): Map<string, LcmSession> {
    return this.sessions;
  }

  async snapshot(sessionId: string): Promise<SessionSnapshot | undefined> {
    const s = this.sessions.get(sessionId);
    if (!s) return undefined;

    const ctx = await s.getContext();
    const summaryCount = ctx.filter((i) => i.kind === "summary").length;
    const messageCount = ctx.filter((i) => i.kind === "message").length;
    const archived = await s.dag.getArchived(s.session.id);

    return {
      sessionId: s.session.id,
      createdAt: s.session.createdAt.toISOString(),
      activeTokenCount: await s.getTokenCount(),
      totalMessages: await s.store.size(),
      activeSummaries: summaryCount,
      archivedSummaries: archived.length,
      totalSummaryNodes: await s.dag.size(),
      contextItems: ctx.length,
      isActive: s.session.id === this.activeId,
    };
  }

  async overview(): Promise<DashboardOverview> {
    const sessions: SessionSnapshot[] = [];
    for (const [id] of this.sessions) {
      const snap = await this.snapshot(id);
      if (snap) sessions.push(snap);
    }
    return {
      timestamp: new Date().toISOString(),
      sessionCount: this.sessions.size,
      activeSessionId: this.activeId,
      sessions,
    };
  }
}

/** Singleton registry shared across MCP server, hooks, and dashboard. */
export const registry = new SessionRegistry();
