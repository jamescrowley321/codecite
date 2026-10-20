-- A chunk_fts row's rowid is its chunk.id; contentless-delete FTS rows are deleted by rowid and can only be updated as a whole row.

CREATE TABLE meta     (key TEXT PRIMARY KEY, value TEXT);          -- schema_version, indexed_at, manifest_hash

CREATE TABLE source (
  id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL,                                               -- local | git | document
  url TEXT NOT NULL,                                                -- repo URL, local path or document URL
  revision TEXT NOT NULL,                                           -- commit SHA, or sha256 of a document
  is_current INTEGER NOT NULL DEFAULT 1,                            -- 0 for an older pinned version of a document
  obsoleted_by TEXT, updated_by TEXT,                               -- hand-written in the manifest
  license_spdx TEXT, license_class TEXT NOT NULL,                   -- adapt | approach_only | quote | excluded
  indexed_at TEXT NOT NULL
);

CREATE TABLE document (
  id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES source(id),
  path TEXT NOT NULL, language TEXT, content_hash BLOB NOT NULL, size_bytes INTEGER,
  UNIQUE (source_id, path)
);

CREATE TABLE chunk (
  id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES document(id),
  start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
  kind TEXT NOT NULL,                                               -- function | class | section | step | window
  symbol_id TEXT REFERENCES symbol(id),
  number TEXT, anchor TEXT, title TEXT,                             -- spec sections and steps: "3.1.3.7", "IDTokenValidation"
  parent_id INTEGER REFERENCES chunk(id),                           -- enclosing section of a section or step
  strength TEXT,                                                    -- requirement keywords found, e.g. "MUST,SHOULD"
  text TEXT NOT NULL,
  terms TEXT NOT NULL                                               -- identifiers split for search
);

CREATE VIRTUAL TABLE chunk_fts USING fts5(
  symbol, path, terms, content='', contentless_delete=1, tokenize='unicode61'
);

CREATE TABLE symbol (
  id TEXT PRIMARY KEY,                                              -- SCIP symbol string
  display_name TEXT NOT NULL, kind TEXT,
  document_id INTEGER REFERENCES document(id), start_line INTEGER, end_line INTEGER
);

CREATE TABLE edge (
  src TEXT NOT NULL, dst TEXT NOT NULL,
  kind TEXT NOT NULL,                                               -- defines | references | calls | imports | inherits | documents
  provider TEXT NOT NULL,                                           -- treesitter | scip | lsp | codegraph | graphify
  confidence INTEGER NOT NULL,                                      -- 3 exact, 2 extracted, 1 inferred, 0 ambiguous
  evidence_doc INTEGER REFERENCES document(id), evidence_line INTEGER,
  PRIMARY KEY (src, dst, kind, provider)
);
CREATE INDEX edge_dst ON edge(dst, kind);
CREATE INDEX symbol_name ON symbol(display_name);
