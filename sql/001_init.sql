-- bacon-lcm schema
-- Run this migration against your Postgres database to set up the LCM tables.

BEGIN;

-- ---------------------------------------------------------------------------
-- Messages (Immutable Store)
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS lcm_messages (
  id              TEXT        PRIMARY KEY,
  session_id      TEXT        NOT NULL,
  role            TEXT        NOT NULL CHECK (role IN ('user', 'assistant', 'tool', 'system')),
  content         TEXT        NOT NULL,
  sequence_number INTEGER     NOT NULL,
  token_count     INTEGER     NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  metadata        JSONB,

  UNIQUE (session_id, sequence_number)
);

CREATE INDEX IF NOT EXISTS idx_lcm_messages_session
  ON lcm_messages (session_id, sequence_number);

-- ---------------------------------------------------------------------------
-- Summary Nodes (DAG)
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS lcm_summary_nodes (
  id                  TEXT        PRIMARY KEY,
  session_id          TEXT        NOT NULL,
  level               TEXT        NOT NULL CHECK (level IN ('leaf', 'condensed', 'emergency')),
  content             TEXT        NOT NULL,
  token_count         INTEGER     NOT NULL,
  created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  source_message_ids  TEXT[]      NOT NULL DEFAULT '{}',
  source_node_ids     TEXT[]      NOT NULL DEFAULT '{}',
  is_active           BOOLEAN     NOT NULL DEFAULT TRUE,
  is_archived         BOOLEAN     NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_lcm_summary_nodes_session
  ON lcm_summary_nodes (session_id);

CREATE INDEX IF NOT EXISTS idx_lcm_summary_nodes_active
  ON lcm_summary_nodes (session_id) WHERE is_active = TRUE AND is_archived = FALSE;

CREATE INDEX IF NOT EXISTS idx_lcm_summary_nodes_archived
  ON lcm_summary_nodes (session_id) WHERE is_archived = TRUE;

-- ---------------------------------------------------------------------------
-- Sessions (optional — useful for listing / restoring sessions)
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS lcm_sessions (
  id                  TEXT        PRIMARY KEY,
  created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  active_token_count  INTEGER     NOT NULL DEFAULT 0
);

COMMIT;
