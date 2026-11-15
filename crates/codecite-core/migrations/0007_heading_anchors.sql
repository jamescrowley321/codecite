-- Markdown sections carry their heading's GitHub-style anchor in `chunk.anchor`.

-- Markdown files indexed before anchors have none: an empty hash never matches, so the next index run splits them again. A guidance document keeps no text and is split again on every run anyway.
UPDATE document SET content_hash = x''
  WHERE language = 'markdown' AND text IS NOT NULL;
