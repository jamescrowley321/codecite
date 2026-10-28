-- Spec chunks get their own full-text table, stemmed with Porter, so a search for "authenticates" finds "authentication". Code stays in chunk_fts, unstemmed, until stemming code is measured. A spec_fts row's rowid is its chunk.id, as in chunk_fts.
CREATE VIRTUAL TABLE spec_fts USING fts5(
  symbol, path, terms, content='', contentless_delete=1, tokenize='porter unicode61'
);

CREATE TRIGGER spec_fts_delete AFTER DELETE ON chunk BEGIN
  DELETE FROM spec_fts WHERE rowid = old.id;
END;

-- Spec chunks leave chunk_fts. A contentless table can't give their terms back, so an empty hash makes the next index run rewrite each spec document into spec_fts.
DELETE FROM chunk_fts WHERE rowid IN (
  SELECT c.id FROM chunk c JOIN document d ON d.id = c.document_id
  JOIN source s ON s.id = d.source_id WHERE s.kind = 'document'
);
UPDATE document SET content_hash = x''
  WHERE source_id IN (SELECT id FROM source WHERE kind = 'document');
