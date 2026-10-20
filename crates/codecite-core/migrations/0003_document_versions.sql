-- Several pinned versions of one document are separate sources: a document's name is unique per revision, any other source's name on its own.
-- Deleting a source or a document deletes everything stored for it: SQLite can't add ON DELETE CASCADE to a column, so these tables are rebuilt.
-- The store turns foreign keys on only after migrating, so the copies below are not checked against tables not yet rebuilt.

CREATE TABLE new_source (
  id INTEGER PRIMARY KEY, name TEXT NOT NULL,
  kind TEXT NOT NULL,
  url TEXT NOT NULL,
  revision TEXT NOT NULL,
  is_current INTEGER NOT NULL DEFAULT 1,
  obsoleted_by TEXT, updated_by TEXT,
  license_spdx TEXT, license_class TEXT NOT NULL,
  indexed_at TEXT NOT NULL
);
INSERT INTO new_source SELECT id, name, kind, url, revision, is_current, obsoleted_by, updated_by, license_spdx, license_class, indexed_at FROM source;
DROP TABLE source;
ALTER TABLE new_source RENAME TO source;
CREATE UNIQUE INDEX source_name ON source(name) WHERE kind <> 'document';
CREATE UNIQUE INDEX source_version ON source(name, revision) WHERE kind = 'document';

CREATE TABLE new_document (
  id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  path TEXT NOT NULL, language TEXT, content_hash BLOB NOT NULL, size_bytes INTEGER,
  dirty INTEGER NOT NULL DEFAULT 0,
  UNIQUE (source_id, path)
);
INSERT INTO new_document SELECT id, source_id, path, language, content_hash, size_bytes, dirty FROM document;
DROP TABLE document;
ALTER TABLE new_document RENAME TO document;

CREATE TABLE new_symbol (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL, kind TEXT,
  document_id INTEGER REFERENCES document(id) ON DELETE CASCADE, start_line INTEGER, end_line INTEGER
);
INSERT INTO new_symbol SELECT id, display_name, kind, document_id, start_line, end_line FROM symbol;
DROP TABLE symbol;
ALTER TABLE new_symbol RENAME TO symbol;
CREATE INDEX symbol_name ON symbol(display_name);
CREATE INDEX symbol_document ON symbol(document_id);

CREATE TABLE new_chunk (
  id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES document(id) ON DELETE CASCADE,
  start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
  kind TEXT NOT NULL,
  symbol_id TEXT REFERENCES symbol(id) ON DELETE SET NULL,
  number TEXT, anchor TEXT, title TEXT,
  parent_id INTEGER REFERENCES chunk(id) ON DELETE CASCADE,
  strength TEXT,
  text TEXT NOT NULL,
  terms TEXT NOT NULL
);
INSERT INTO new_chunk SELECT id, document_id, start_line, end_line, kind, symbol_id, number, anchor, title, parent_id, strength, text, terms FROM chunk;
DROP TABLE chunk;
ALTER TABLE new_chunk RENAME TO chunk;
-- Every foreign key is indexed, so a cascading delete looks its children up instead of scanning.
CREATE INDEX chunk_document ON chunk(document_id);
CREATE INDEX chunk_parent ON chunk(parent_id);
CREATE INDEX chunk_symbol ON chunk(symbol_id);

-- A contentless full-text table can't be a foreign key's child, so a trigger deletes a chunk's row, by rowid, with the chunk.
CREATE TRIGGER chunk_fts_delete AFTER DELETE ON chunk BEGIN
  DELETE FROM chunk_fts WHERE rowid = old.id;
END;

CREATE TABLE new_edge (
  src TEXT NOT NULL, dst TEXT NOT NULL,
  kind TEXT NOT NULL,
  provider TEXT NOT NULL,
  confidence INTEGER NOT NULL,
  evidence_doc INTEGER REFERENCES document(id) ON DELETE CASCADE, evidence_line INTEGER,
  PRIMARY KEY (src, dst, kind, provider)
);
INSERT INTO new_edge SELECT src, dst, kind, provider, confidence, evidence_doc, evidence_line FROM edge;
DROP TABLE edge;
ALTER TABLE new_edge RENAME TO edge;
CREATE INDEX edge_dst ON edge(dst, kind);
CREATE INDEX edge_evidence ON edge(evidence_doc);
