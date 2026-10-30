-- Search ranks every chunk in one query over chunk_fts, so spec_fts, the stemmed table that held spec chunks a second time, has no reader. Drop it and its trigger rather than write rows nothing reads. Versions 5 and 8 still fill it on the way up from an older corpus; this file drops it after them.
DROP TRIGGER IF EXISTS spec_fts_delete;
DROP TABLE IF EXISTS spec_fts;
