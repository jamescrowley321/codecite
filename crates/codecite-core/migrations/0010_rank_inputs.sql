-- Search computed a document's role weight and each symbol's name key in Rust for every candidate it ranked, so a query cost grew with its matches (#251). Store both at write time. Store::open_read_write fills them for the rows already stored right after this file runs, in the same transaction, from the same Rust functions the write path uses.
ALTER TABLE document ADD COLUMN role TEXT;          -- source | test | example | docs | spec
ALTER TABLE document ADD COLUMN role_weight REAL;   -- the role's factor on a BM25 score
ALTER TABLE symbol ADD COLUMN name_key TEXT;        -- display_name lowercased, without _ or -
CREATE INDEX symbol_name_key ON symbol(name_key);
