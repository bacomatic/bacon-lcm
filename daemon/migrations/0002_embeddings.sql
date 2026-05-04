-- daemon/migrations/0002_embeddings.sql
BEGIN;

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS lcm_embeddings (
    id          UUID        PRIMARY KEY,
    session_id  UUID        NOT NULL REFERENCES lcm_sessions(id) ON DELETE CASCADE,
    content     TEXT        NOT NULL,
    embedding   vector      NOT NULL,
    metadata    JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_lcm_embeddings_session
    ON lcm_embeddings (session_id);

COMMIT;
