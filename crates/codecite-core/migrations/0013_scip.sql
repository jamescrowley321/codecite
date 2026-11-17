-- SCIP: precise symbols and the references between them, read from a SCIP index a source's manifest entry names.

-- Which provider a symbol's ID comes from: `treesitter`, or `scip` when a SCIP definition replaced the tree-sitter ID or found a definition tree-sitter doesn't chunk.
ALTER TABLE symbol ADD COLUMN provider TEXT NOT NULL DEFAULT 'treesitter';
-- The tree-sitter ID a SCIP symbol replaced, so the next index run can undo the SCIP layer before applying the current one.
ALTER TABLE symbol ADD COLUMN local_id TEXT;

-- Where a symbol occurs: one row per SCIP occurrence, definitions included. `edge` holds one row per pair of symbols; this holds every site, which `references` lists.
CREATE TABLE occurrence (
  symbol_id TEXT NOT NULL,                                          -- a SCIP symbol, perhaps defined outside the corpus
  document_id INTEGER NOT NULL REFERENCES document(id) ON DELETE CASCADE,
  start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
  roles INTEGER NOT NULL,                                           -- SCIP SymbolRole bits: 1 definition, 2 import, 4 write, 8 read, 16 generated, 32 test
  enclosing TEXT,                                                   -- the innermost definition holding it
  provider TEXT NOT NULL,
  confidence INTEGER NOT NULL
);
CREATE INDEX occurrence_symbol ON occurrence(symbol_id);
CREATE INDEX occurrence_document ON occurrence(document_id);
