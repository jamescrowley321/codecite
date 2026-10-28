-- Specs are searched in their own full-text table, stemmed with Porter, so a search for "authenticates" finds "authentication". A spec_fts row's rowid is its chunk.id, as in chunk_fts. Spec chunks keep their chunk_fts rows too: code ranks against chunk_fts, and its BM25 statistics count every chunk, so taking them out would move code results.
CREATE VIRTUAL TABLE spec_fts USING fts5(
  symbol, path, terms, content='', contentless_delete=1, tokenize='porter unicode61'
);

CREATE TRIGGER spec_fts_delete AFTER DELETE ON chunk BEGIN
  DELETE FROM spec_fts WHERE rowid = old.id;
END;

-- A contentless table can't give a chunk's terms back, so an empty hash makes the next index run rewrite each spec document, into both tables.
UPDATE document SET content_hash = x''
  WHERE source_id IN (SELECT id FROM source WHERE kind = 'document');
