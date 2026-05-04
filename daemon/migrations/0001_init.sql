-- daemon/migrations/0001_init.sql
BEGIN;

CREATE TABLE IF NOT EXISTS lcm_sessions (
    id          UUID        PRIMARY KEY,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata    JSONB       NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS lcm_messages (
    id              UUID        PRIMARY KEY,
    session_id      UUID        NOT NULL REFERENCES lcm_sessions(id) ON DELETE CASCADE,
    role            TEXT        NOT NULL CHECK (role IN ('user','assistant','system','tool')),
    content         TEXT        NOT NULL,
    sequence_number INTEGER     NOT NULL,
    token_count     INTEGER     NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata        JSONB       NOT NULL DEFAULT '{}',
    UNIQUE (session_id, sequence_number)
);

CREATE INDEX IF NOT EXISTS idx_lcm_messages_session
    ON lcm_messages (session_id, sequence_number);

CREATE TABLE IF NOT EXISTS lcm_summary_nodes (
    id          UUID        PRIMARY KEY,
    session_id  UUID        NOT NULL REFERENCES lcm_sessions(id) ON DELETE CASCADE,
    level       TEXT        NOT NULL CHECK (level IN ('leaf','condensed','emergency')),
    content     TEXT        NOT NULL,
    token_count INTEGER     NOT NULL,
    lineage     JSONB       NOT NULL DEFAULT '[]',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata    JSONB       NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_lcm_summary_nodes_session
    ON lcm_summary_nodes (session_id);

CREATE INDEX IF NOT EXISTS idx_lcm_summary_nodes_level
    ON lcm_summary_nodes (session_id, level);

COMMIT;
