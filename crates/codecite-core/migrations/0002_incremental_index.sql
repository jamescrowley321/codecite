-- Re-indexing replaces a document's chunks and symbols, so both are looked up by document.

-- Whether a local source's file differed from HEAD (modified, staged or untracked) when it was indexed.
ALTER TABLE document ADD COLUMN dirty INTEGER NOT NULL DEFAULT 0;

CREATE INDEX chunk_document ON chunk(document_id);
CREATE INDEX symbol_document ON symbol(document_id);
CREATE INDEX edge_evidence ON edge(evidence_doc);
