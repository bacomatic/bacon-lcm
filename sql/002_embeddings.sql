-- bacon-lcm embeddings schema (requires pgvector extension)
-- Run this migration after 001_init.sql.

BEGIN;

CREATE EXTENSION IF NOT EXISTS vector;

-- ---------------------------------------------------------------------------
-- Embeddings (vector store for semantic search)
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS lcm_embeddings (
  id          TEXT        PRIMARY KEY,
  session_id  TEXT        NOT NULL,
  source_type TEXT        NOT NULL CHECK (source_type IN ('message', 'summary')),
  source_id   TEXT        NOT NULL,
  content     TEXT        NOT NULL,
  embedding   vector(1536),
  created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

  UNIQUE (source_type, source_id)
);

CREATE INDEX IF NOT EXISTS idx_lcm_embeddings_session
  ON lcm_embeddings (session_id);

-- HNSW index for cosine similarity search (fast approximate nearest neighbor)
CREATE INDEX IF NOT EXISTS idx_lcm_embeddings_vector
  ON lcm_embeddings USING hnsw (embedding vector_cosine_ops);

COMMIT;
