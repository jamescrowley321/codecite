-- Before SQLite 3.54, deleting a row from a contentless_delete FTS5 table left it in the table's totals, so a reindexed corpus ranked by wrong BM25 statistics. Empty both tables, totals included; Store::open_read_write refills them from chunk right after this file runs, in the same transaction.
INSERT INTO chunk_fts (chunk_fts) VALUES ('delete-all');
INSERT INTO spec_fts (spec_fts) VALUES ('delete-all');
