-- A source's own role globs: the JSON of its manifest's `[source.roles]` table, which replaces the path conventions for the roles it names. NULL keeps every convention.
ALTER TABLE source ADD COLUMN role_globs TEXT;
