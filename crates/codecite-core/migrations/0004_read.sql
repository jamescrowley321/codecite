-- What `read` needs to answer from the corpus alone, without the manifest or the source tree.

-- The names people write for documents (`OIDC Core`), normalized as `SpecAliases` normalizes them, and the document each names. Each index run replaces them.
CREATE TABLE alias (alias TEXT PRIMARY KEY, name TEXT NOT NULL);

-- A code file's text as indexed, so `read` returns the exact lines; chunks leave out lines without a word between definitions. NULL for a spec, whose sections and steps are its text.
ALTER TABLE document ADD COLUMN text TEXT;

-- Code files indexed before this column have no text: an empty hash never matches, so the next index run rewrites them.
UPDATE document SET content_hash = x''
  WHERE source_id IN (SELECT id FROM source WHERE kind <> 'document');
