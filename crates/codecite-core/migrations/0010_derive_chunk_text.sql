-- A chunk stores only what can't be derived (#249). `terms` goes: it was always `text_terms(text)`, and chunk_fts already holds it. `text` may be NULL: a chunk of a document that stores its text is a line range of it, so readers cut it from `document.text`.
-- SQLite can't drop NOT NULL from a column, so the table is rebuilt, keeping every chunk's ID, which its chunk_fts row and its children name. Store::open_read_write then clears the text of every chunk its document's text holds, in the same transaction, since lines are counted in Rust.

CREATE TABLE new_chunk (
  id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES document(id) ON DELETE CASCADE,
  start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
  kind TEXT NOT NULL,
  symbol_id TEXT REFERENCES symbol(id) ON DELETE SET NULL,
  number TEXT, anchor TEXT, title TEXT,
  parent_id INTEGER REFERENCES chunk(id) ON DELETE CASCADE,
  strength TEXT,
  text TEXT                                                         -- NULL when it is lines start_line..end_line of document.text
);
INSERT INTO new_chunk SELECT id, document_id, start_line, end_line, kind, symbol_id, number, anchor, title, parent_id, strength, text FROM chunk;
DROP TABLE chunk;
ALTER TABLE new_chunk RENAME TO chunk;
CREATE INDEX chunk_document ON chunk(document_id);
CREATE INDEX chunk_parent ON chunk(parent_id);
CREATE INDEX chunk_symbol ON chunk(symbol_id);

CREATE TRIGGER chunk_fts_delete AFTER DELETE ON chunk BEGIN
  DELETE FROM chunk_fts WHERE rowid = old.id;
END;
