-- Guidance: a team's rules, indexed as a document's sections, and the files each rule governs, so `context` can push the rules for a file.

-- 1 for a guidance document, whose sections are rules.
ALTER TABLE source ADD COLUMN guidance INTEGER NOT NULL DEFAULT 0;

-- What one rule of a guidance document applies to, as its section number names it. A rule with no rows applies to every file; one with rows applies to a file that matches every kind it has, and a kind matches when any of its values does. Each index run replaces a source's rows, since scopes can live in the manifest.
CREATE TABLE rule_scope (
  source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  number TEXT NOT NULL,                                             -- the rule's section number
  kind TEXT NOT NULL,                                               -- path | language | import
  value TEXT NOT NULL,                                              -- a glob, a language name or a module
  PRIMARY KEY (source_id, number, kind, value)
);
