-- Which build's rules wrote each document's derived rows (#252): its chunks by the chunker at `chunker_version`, their full-text terms by the rules at `terms_version`. An index run splits a document again when either is behind the build's CHUNKER_VERSION or TERMS_VERSION, so a migration that changes derived data sets them to 0 for the documents it affects instead of clearing their `content_hash`.
-- Every document stored before this version was written by version 1 of both.
ALTER TABLE document ADD COLUMN chunker_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE document ADD COLUMN terms_version INTEGER NOT NULL DEFAULT 0;
UPDATE document SET chunker_version = 1, terms_version = 1;

-- The FTS5 tokenizer chunk_fts was created with, FTS_TOKENIZER in the build that made it. Changing it means rebuilding chunk_fts, which only a migration can do.
INSERT INTO meta (key, value) VALUES ('tokenizer', 'unicode61')
  ON CONFLICT (key) DO UPDATE SET value = excluded.value;
