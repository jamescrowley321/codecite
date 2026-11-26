-- Search joins every candidate chunk's document for its role and weight, which sat after the file's text in the row, so SQLite walked each text's overflow pages to reach them; with the page cache cold that was most of a search's time (#308). Each document's text moves to its own table, which only the readers that cite text join.
CREATE TABLE document_text (
  document_id INTEGER PRIMARY KEY REFERENCES document(id) ON DELETE CASCADE,
  text TEXT NOT NULL
);
INSERT INTO document_text (document_id, text) SELECT id, text FROM document WHERE text IS NOT NULL;
ALTER TABLE document DROP COLUMN text;
