-- One row per `codecite index` run, so a corpus can say which manifest and which codecite build produced it. A run writes everything in one transaction; its row is written before that transaction and updated after it, so a failed run is recorded too.
CREATE TABLE index_run (
  id INTEGER PRIMARY KEY,
  started_at TEXT NOT NULL, finished_at TEXT,                       -- finished_at is NULL while running, or for a run that was killed
  status TEXT NOT NULL,                                             -- running | succeeded | failed
  codecite_version TEXT NOT NULL,
  manifest_sha256 TEXT NOT NULL,
  chunker_version INTEGER NOT NULL, terms_version INTEGER NOT NULL, tokenizer TEXT NOT NULL,
  sources INTEGER, written INTEGER, unchanged INTEGER, removed INTEGER, chunks INTEGER,
  error TEXT                                                        -- why a failed run failed
);

-- The run that last published each source.
ALTER TABLE source ADD COLUMN index_run_id INTEGER REFERENCES index_run(id);
