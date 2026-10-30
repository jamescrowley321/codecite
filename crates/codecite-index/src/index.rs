//! Indexing a corpus: walk each local source and each git source's clone at
//! its pinned commit, chunk the files whose content changed, split each new document version into its sections and steps,
//! and write them into the store, one transaction per document.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use codecite_core::{
    ChunkRecord, DocumentRecord, RuleScope, SourceRecord, SpecAliases, Store, SymbolRecord,
};
use sha2::{Digest, Sha256};

use crate::{
    Checkout, Chunker, Clones, DocumentSource, Error, Fetcher, GitSource, GuidanceError,
    GuidanceRule, GuidanceSource, Language, LicenseClass, LocalSource, LocalTree,
    MAX_DOCUMENT_SIZE, Manifest, Result, SkipReason, Source, SpecError, SpecSection, WalkOptions,
    document::split_document, split_guidance, split_spec, walk_local,
};

/// What an index run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexSummary {
    /// Documents written: new, or with changed content.
    pub written: usize,
    /// Documents left as they were because their content hash is unchanged.
    pub unchanged: usize,
    /// Documents deleted because their file is gone or now skipped.
    pub removed: usize,
    /// Chunks written, over every written document.
    pub chunks: usize,
    /// Document sources downloaded and checked against their pin.
    pub downloaded: usize,
    /// Document sources read from the cache.
    pub cached: usize,
    /// Files not indexed, counted by reason.
    pub skipped: BTreeMap<SkipReason, usize>,
    /// Manifest sources not indexed, with the reason.
    pub skipped_sources: Vec<(String, &'static str)>,
    /// Sources deleted from the store because the run did not index them.
    pub removed_sources: Vec<String>,
    /// Guidance read from a `path` with no `sha256` pin, whose rules reach
    /// agents whatever the file says now.
    pub unpinned_guidance: Vec<String>,
    /// How long the run took.
    pub elapsed: Duration,
}

/// Fetches every document source and git source in `manifest` and indexes
/// them and every local source into `store`.
///
/// Documents are fetched first, from `fetcher`'s cache when they are there,
/// so a document whose bytes don't match its pinned `sha256` stops the run
/// before anything is written. So is each git source's pinned commit, into
/// its clone in `clones`, so a commit the repository lacks stops the run
/// too. A git source is then checked out at its commit and walked like a
/// local checkout, and its files are never `dirty`. Each pinned version of a document is its own
/// source, split into section and step chunks when it is first indexed.
///
/// A file whose content hash matches the stored one is not rewritten, and a
/// stored file that is gone, or is now skipped, is deleted, so re-indexing
/// a git source at a new commit rewrites only the files that changed between
/// the two commits. Sources marked `excluded` are reported in
/// [`IndexSummary::skipped_sources`]. Every source the run did not index, a document version no
/// longer pinned included, is then deleted from the store.
///
/// A reindex leaves `store` as a fresh index of the same pins would: the
/// same rows and the same full-text totals, so search ranks alike.
pub fn index_corpus(
    store: &mut Store,
    manifest: &Manifest,
    options: &WalkOptions,
    fetcher: &Fetcher,
    clones: &Clones,
) -> Result<IndexSummary> {
    let started = Instant::now();
    let mut summary = IndexSummary::default();
    let mut documents = Vec::new();
    for source in &manifest.sources {
        if let Source::Document(doc) = source
            && doc.license_class != LicenseClass::Excluded
        {
            let fetched = fetcher.fetch_pinned(doc)?;
            if fetched.from_cache {
                summary.cached += 1;
            } else {
                summary.downloaded += 1;
            }
            documents.push((doc, fetched.bytes));
        }
    }
    // Each guidance document with its bytes and the revision it is cited
    // at: its pin when fetched, the sha256 of the file as read when local.
    let mut guidance = Vec::new();
    for source in &manifest.sources {
        let Source::Guidance(rules) = source else {
            continue;
        };
        if rules.license_class == LicenseClass::Excluded {
            continue;
        }
        if let Some(doc) = rules.as_document() {
            let fetched = fetcher.fetch_pinned(&doc)?;
            if fetched.from_cache {
                summary.cached += 1;
            } else {
                summary.downloaded += 1;
            }
            let split = prepare_guidance(rules, &fetched.bytes)?;
            guidance.push((rules, fetched.bytes, doc.sha256, split));
        } else if let Some(path) = &rules.path {
            let bytes = read_guidance(path)?;
            let revision = crate::fetch::hex(&Sha256::digest(&bytes));
            if rules.sha256.is_none() {
                summary.unpinned_guidance.push(rules.name.clone());
            }
            if let Some(pinned) = rules.sha256.as_ref().filter(|p| **p != revision) {
                return Err(Error::GuidanceChanged {
                    name: rules.name.clone(),
                    path: path.clone(),
                    expected: pinned.clone(),
                    actual: revision,
                });
            }
            // Split and checked before anything is written, so a bad
            // guidance file can't leave the store half updated.
            let split = prepare_guidance(rules, &bytes)?;
            guidance.push((rules, bytes, revision, split));
        }
    }
    for source in &manifest.sources {
        if let Source::Git(git) = source
            && git.license_class != LicenseClass::Excluded
        {
            clones.fetch(git)?;
        }
    }
    let aliases = document_aliases(manifest)?;
    let mut chunker = Chunker::new()?;
    let mut indexed: Vec<String> = Vec::new();
    let mut keep: Vec<i64> = Vec::new();
    for source in &manifest.sources {
        let walked = match source {
            Source::Local(local) if local.license_class != LicenseClass::Excluded => {
                let tree = walk_local(local, options)?;
                Walked {
                    name: source_name(local, &tree),
                    kind: "local",
                    url: tree.root.to_string_lossy().into_owned(),
                    license: local.license.as_deref(),
                    license_class: local.license_class,
                    tree,
                    checkout: None,
                }
            }
            Source::Git(git) if git.license_class != LicenseClass::Excluded => {
                let (tree, checkout) = walk_git(clones, git, options)?;
                Walked {
                    name: git_name(git),
                    kind: "git",
                    url: git.url.clone(),
                    license: git.license.as_deref(),
                    license_class: git.license_class,
                    tree,
                    checkout: Some(checkout),
                }
            }
            Source::Local(local) => {
                let name = local
                    .name
                    .clone()
                    .unwrap_or_else(|| local.path.display().to_string());
                summary.skipped_sources.push((name, "use = \"excluded\""));
                continue;
            }
            Source::Git(git) => {
                summary
                    .skipped_sources
                    .push((git_name(git), "use = \"excluded\""));
                continue;
            }
            Source::Document(doc) => {
                if doc.license_class == LicenseClass::Excluded {
                    summary
                        .skipped_sources
                        .push((doc.name.clone(), "use = \"excluded\""));
                }
                continue;
            }
            Source::Guidance(rules) => {
                if rules.license_class == LicenseClass::Excluded {
                    summary
                        .skipped_sources
                        .push((rules.name.clone(), "use = \"excluded\""));
                }
                continue;
            }
        };
        if indexed.contains(&walked.name) {
            return Err(Error::DuplicateSource { name: walked.name });
        }
        keep.push(index_tree(store, &mut chunker, &walked, &mut summary)?);
        indexed.push(walked.name);
    }
    for (doc, bytes) in &documents {
        // A document's versions share its name, but no other source may.
        if indexed.contains(&doc.name) {
            return Err(Error::DuplicateSource {
                name: doc.name.clone(),
            });
        }
        keep.push(index_document(store, doc, bytes, &mut summary)?);
    }
    for (rules, bytes, revision, split) in &guidance {
        // Guidance is stored as a document, so its name may be no other
        // source's.
        if indexed.contains(&rules.name) || documents.iter().any(|(d, _)| d.name == rules.name) {
            return Err(Error::DuplicateSource {
                name: rules.name.clone(),
            });
        }
        indexed.push(rules.name.clone());
        keep.push(index_guidance(
            store,
            rules,
            bytes,
            revision,
            split,
            &mut summary,
        )?);
    }
    summary.removed_sources = store.delete_sources_except(&keep)?;
    store.set_aliases(&aliases)?;
    store.set_indexed_at(manifest.max_age_days)?;
    summary.elapsed = started.elapsed();
    Ok(summary)
}

/// The aliases of every document the run indexes. An alias that names two
/// documents is [`Error::DuplicateAlias`], found before anything is written.
fn document_aliases(manifest: &Manifest) -> Result<SpecAliases> {
    let mut aliases = SpecAliases::new();
    for source in &manifest.sources {
        let (name, names) = match source {
            Source::Document(doc) => (&doc.name, &doc.aliases),
            Source::Guidance(rules) => (&rules.name, &rules.aliases),
            _ => continue,
        };
        if source.license_class() == LicenseClass::Excluded {
            continue;
        }
        for alias in names {
            match aliases.get(alias) {
                Some(first) if first != name => {
                    return Err(Error::DuplicateAlias {
                        alias: alias.clone(),
                        first: first.into(),
                        second: name.clone(),
                    });
                }
                _ => aliases.insert(alias, name),
            }
        }
    }
    Ok(aliases)
}

/// The manifest's `name`, else the checkout directory's name.
fn source_name(local: &LocalSource, tree: &LocalTree) -> String {
    local.name.clone().unwrap_or_else(|| {
        tree.root
            .file_name()
            .map_or_else(|| "local".to_owned(), |n| n.to_string_lossy().into_owned())
    })
}

/// The manifest's `name`, else the URL's last path segment without `.git`:
/// `identity-model` for `https://github.com/acme/identity-model.git`.
fn git_name(git: &GitSource) -> String {
    git.name.clone().unwrap_or_else(|| {
        let url = git.url.trim_end_matches('/');
        let last = url.rsplit(['/', ':']).next().unwrap_or(url);
        let stem = last.strip_suffix(".git").unwrap_or(last);
        if stem.is_empty() { "git" } else { stem }.to_owned()
    })
}

/// Checks a git source out at its pinned commit and walks the clone. The
/// work tree is exactly the commit, so no file is `dirty`. The returned
/// [`Checkout`] holds the clone's lock; files are read from the tree while
/// it lives.
fn walk_git(
    clones: &Clones,
    git: &GitSource,
    options: &WalkOptions,
) -> Result<(LocalTree, Checkout)> {
    let locked = clones.checkout(git)?;
    let checkout = LocalSource {
        name: git.name.clone(),
        path: locked.path().to_owned(),
        license: git.license.clone(),
        license_class: git.license_class,
        include: git.include.clone(),
        exclude: git.exclude.clone(),
    };
    let mut tree = walk_local(&checkout, options)?;
    for file in &mut tree.files {
        file.dirty = false;
    }
    Ok((tree, locked))
}

/// A local or git source, walked and ready to write.
struct Walked<'a> {
    /// The name results cite.
    name: String,
    /// `local` or `git`.
    kind: &'static str,
    /// The checkout's root for a local source, the repository URL for a git
    /// source.
    url: String,
    license: Option<&'a str>,
    license_class: LicenseClass,
    /// The walk. Its HEAD is the commit results cite: a local checkout's
    /// HEAD, or a git source's pinned commit.
    tree: LocalTree,
    /// A git source's clone, locked until every file is read, so another
    /// run can't check out a different commit under this one.
    #[expect(dead_code, reason = "held for its lock, never read")]
    checkout: Option<Checkout>,
}

/// Writes one walked source into the store and returns its ID.
fn index_tree(
    store: &mut Store,
    chunker: &mut Chunker,
    walked: &Walked<'_>,
    summary: &mut IndexSummary,
) -> Result<i64> {
    let (name, tree) = (walked.name.as_str(), &walked.tree);
    let source_id = store.upsert_source(&SourceRecord {
        name,
        kind: walked.kind,
        url: &walked.url,
        revision: &tree.head,
        license_spdx: walked.license,
        license_class: walked.license_class.as_str(),
        is_current: true,
        obsoleted_by: None,
        updated_by: None,
    })?;
    for (reason, count) in tree.skip_counts() {
        *summary.skipped.entry(reason).or_default() += count;
    }

    let stored = store.document_hashes(source_id)?;
    let mut kept = HashSet::new();
    for file in &tree.files {
        let bytes = match read(&tree.root, &file.path)? {
            Some(bytes) => bytes,
            None => {
                *summary.skipped.entry(SkipReason::Deleted).or_default() += 1;
                continue;
            }
        };
        let hash = Sha256::digest(&bytes);
        if stored
            .get(&file.path)
            .is_some_and(|old| old[..] == hash[..])
        {
            store.set_dirty(source_id, &file.path, file.dirty)?;
            kept.insert(file.path.as_str());
            summary.unchanged += 1;
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            *summary
                .skipped
                .entry(SkipReason::NonUtf8Content)
                .or_default() += 1;
            continue;
        };
        let chunks = chunker.chunk(&file.path, &text);
        let symbol_ids: Vec<Option<String>> = chunks
            .iter()
            .map(|c| c.symbol.as_ref().map(|s| s.scip_id(name, &file.path)))
            .collect();
        let document = DocumentRecord {
            path: &file.path,
            language: Language::detect(&file.path, &text).map(Language::name),
            content_hash: hash.as_slice(),
            size_bytes: text.len() as u64,
            dirty: file.dirty,
            text: Some(&text),
            chunks: chunks
                .iter()
                .zip(symbol_ids)
                .map(|(c, id)| ChunkRecord {
                    kind: c.kind.as_str(),
                    start_line: c.start_line,
                    end_line: c.end_line,
                    text: c.text,
                    number: None,
                    anchor: c.anchor.as_deref(),
                    title: c.title.as_deref(),
                    parent: None,
                    strength: &[],
                    symbol: c.symbol.as_ref().zip(id).map(|(s, id)| SymbolRecord {
                        id,
                        display_name: &s.name,
                        kind: s.kind.as_str(),
                    }),
                })
                .collect(),
        };
        summary.chunks += store.write_document(source_id, &document)?;
        summary.written += 1;
        kept.insert(file.path.as_str());
    }

    for path in stored.keys().filter(|p| !kept.contains(p.as_str())) {
        store.delete_document(source_id, path)?;
        summary.removed += 1;
    }
    Ok(source_id)
}

/// Writes one pinned document version into the store and returns its
/// source's ID.
///
/// The source row is updated on every run, since `current` and the
/// relations live in the manifest. The document is split and written only
/// when it isn't stored yet: its revision is its content's hash, so a
/// stored version can't have changed.
fn index_document(
    store: &mut Store,
    doc: &DocumentSource,
    bytes: &[u8],
    summary: &mut IndexSummary,
) -> Result<i64> {
    let source_id = store.upsert_source(&SourceRecord {
        name: &doc.name,
        kind: "document",
        url: &doc.url,
        revision: &doc.sha256,
        license_spdx: doc.license.as_deref(),
        license_class: doc.license_class.as_str(),
        is_current: doc.current,
        obsoleted_by: doc.obsoleted_by.as_deref(),
        updated_by: doc.updated_by.as_deref(),
    })?;
    // A spec gives no rules, even when the same version was guidance.
    store.clear_guidance(source_id)?;
    // A document source holds one document, stored under its name.
    let path = doc.name.as_str();
    let hash = Sha256::digest(bytes);
    // The same bytes split the same way, unless the version was stored as
    // guidance: Markdown with no text of its own, split into rules.
    if store
        .document_hashes(source_id)?
        .get(path)
        .is_some_and(|old| old[..] == hash[..])
        && !stored_as_guidance(store, source_id, path)?
    {
        summary.unchanged += 1;
        return Ok(source_id);
    }

    let page = std::str::from_utf8(bytes).map_err(|_| Error::DocumentNotUtf8 {
        name: doc.name.clone(),
    })?;
    let written = match split_spec(page) {
        Ok(spec) => {
            let step_numbers: Vec<Vec<String>> = spec
                .sections
                .iter()
                .map(|s| s.steps.iter().map(|step| step.number.to_string()).collect())
                .collect();
            store.write_document(
                source_id,
                &DocumentRecord {
                    path,
                    language: None,
                    content_hash: hash.as_slice(),
                    size_bytes: bytes.len() as u64,
                    dirty: false,
                    // A spec's text is its sections and steps.
                    text: None,
                    chunks: spec_chunks(&spec.sections, &step_numbers),
                },
            )?
        }
        // Any other page is split at its own headings, like a file in a
        // repository, and keeps the text its lines count in.
        Err(SpecError::UnknownFormat) => {
            let generic = split_document(page, &doc.url);
            store.write_document(
                source_id,
                &DocumentRecord {
                    path,
                    language: Some(generic.format.as_str()),
                    content_hash: hash.as_slice(),
                    size_bytes: bytes.len() as u64,
                    dirty: false,
                    text: Some(&generic.text),
                    chunks: generic
                        .chunks()
                        .iter()
                        .map(|c| ChunkRecord {
                            kind: c.kind.as_str(),
                            start_line: c.start_line,
                            end_line: c.end_line,
                            text: c.text,
                            number: None,
                            anchor: c.anchor.as_deref(),
                            title: c.title.as_deref(),
                            parent: None,
                            strength: &[],
                            symbol: None,
                        })
                        .collect(),
                },
            )?
        }
    };
    summary.chunks += written;
    summary.written += 1;
    Ok(source_id)
}

/// Whether the document at `path` in the source was stored as guidance:
/// Markdown split into rules, with no text of its own.
fn stored_as_guidance(store: &Store, source_id: i64, path: &str) -> Result<bool> {
    Ok(store
        .document_language(source_id, path)?
        .flatten()
        .as_deref()
        == Some(Language::Markdown.name())
        && !store.document_has_text(source_id, path)?)
}

/// Writes one guidance document into the store as a document version of
/// its own, marks it guidance, and stores each rule's scope. Returns its
/// source's ID.
///
/// The document is split on every run, since its rules' scopes come from
/// the manifest as well as from the text, but written only when its bytes
/// changed. A rule's scope is the manifest's `rules."<number>"`, else its
/// own comment's, else its parent rule's, else the manifest's `scope`.
fn index_guidance(
    store: &mut Store,
    rules: &GuidanceSource,
    bytes: &[u8],
    revision: &str,
    split: &[GuidanceRule],
    summary: &mut IndexSummary,
) -> Result<i64> {
    let name = rules.name.as_str();
    let url = match (&rules.url, &rules.path) {
        (Some(url), _) => url.clone(),
        (None, Some(path)) => path.to_string_lossy().into_owned(),
        (None, None) => String::new(),
    };
    let source_id = store.upsert_source(&SourceRecord {
        name,
        kind: "document",
        url: &url,
        revision,
        license_spdx: rules.license.as_deref(),
        license_class: rules.license_class.as_str(),
        is_current: true,
        obsoleted_by: None,
        updated_by: None,
    })?;
    let hash = Sha256::digest(bytes);
    let sections: Vec<SpecSection> = split.iter().map(|r| r.section.clone()).collect();
    // A version stored as a spec or a generic document with the same bytes
    // was split another way, so it is split again.
    if store
        .document_hashes(source_id)?
        .get(name)
        .is_some_and(|old| old[..] == hash[..])
        && stored_as_guidance(store, source_id, name)?
    {
        summary.unchanged += 1;
    } else {
        let no_steps = vec![Vec::new(); sections.len()];
        let document = DocumentRecord {
            path: name,
            language: Some(Language::Markdown.name()),
            content_hash: hash.as_slice(),
            size_bytes: bytes.len() as u64,
            dirty: false,
            // As for a spec, the rules are its text.
            text: None,
            chunks: spec_chunks(&sections, &no_steps),
        };
        summary.chunks += store.write_document(source_id, &document)?;
        summary.written += 1;
    }

    let mut effective: Vec<RuleScope> = Vec::with_capacity(split.len());
    for rule in split {
        let scope = rules
            .rules
            .get(&rule.section.number)
            .or(rule.scope.as_ref())
            .or_else(|| rule.section.parent.map(|p| &effective[p]))
            .unwrap_or(&rules.scope)
            .clone();
        effective.push(scope);
    }
    let scopes: Vec<(&str, &RuleScope)> = split
        .iter()
        .zip(&effective)
        .filter(|(_, scope)| !scope.is_empty())
        .map(|(rule, scope)| (rule.section.number.as_str(), scope))
        .collect();
    store.set_guidance(source_id, &scopes)?;
    Ok(source_id)
}

/// Splits a guidance document into its rules and checks them against the
/// manifest: its `rules."<n>"` must each name a rule, and each rule's own
/// scope must hold valid globs, languages and imports.
fn prepare_guidance(rules: &GuidanceSource, bytes: &[u8]) -> Result<Vec<GuidanceRule>> {
    let name = rules.name.as_str();
    let guidance_error = |source| Error::Guidance {
        name: name.to_owned(),
        source,
    };
    let text = std::str::from_utf8(bytes).map_err(|_| Error::DocumentNotUtf8 {
        name: name.to_owned(),
    })?;
    let split = split_guidance(text).map_err(guidance_error)?;
    for number in rules.rules.keys() {
        if !split.iter().any(|r| &r.section.number == number) {
            return Err(Error::UnknownRule {
                name: name.to_owned(),
                number: number.clone(),
            });
        }
    }
    for rule in &split {
        if let Some(Err(message)) = rule.scope.as_ref().map(RuleScope::check) {
            return Err(guidance_error(GuidanceError::Scope {
                number: rule.section.number.clone(),
                message,
            }));
        }
    }
    Ok(split)
}

/// Reads a local guidance file: a regular file, so a FIFO or device can't
/// block the run, of at most [`MAX_DOCUMENT_SIZE`], the cap fetched
/// documents have, with the read itself stopped past it.
fn read_guidance(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read as _;

    let refuse = |message: &str| Error::ReadFile {
        path: path.to_owned(),
        source: io::Error::other(message.to_owned()),
    };
    let read_error = |source| Error::ReadFile {
        path: path.to_owned(),
        source,
    };
    if !std::fs::metadata(path).map_err(read_error)?.is_file() {
        return Err(refuse("not a regular file"));
    }
    let file = std::fs::File::open(path).map_err(read_error)?;
    if !file.metadata().map_err(read_error)?.is_file() {
        return Err(refuse("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() as u64 > MAX_DOCUMENT_SIZE {
        return Err(refuse("larger than the 16 MiB a guidance document may be"));
    }
    Ok(bytes)
}

/// Each section's chunk followed by its steps' chunks. A section names its
/// enclosing section's chunk as its parent, and a step its section's.
fn spec_chunks<'a>(
    sections: &'a [SpecSection],
    step_numbers: &'a [Vec<String>],
) -> Vec<ChunkRecord<'a>> {
    let mut chunks = Vec::new();
    // The chunk index of each section, by section index. A parent comes
    // before its subsections, so its index is always known here.
    let mut section_chunks = Vec::with_capacity(sections.len());
    for (section, numbers) in sections.iter().zip(step_numbers) {
        let index = chunks.len();
        section_chunks.push(index);
        chunks.push(ChunkRecord {
            kind: "section",
            start_line: section.start_line,
            end_line: section.end_line,
            text: &section.text,
            number: Some(&section.number),
            anchor: Some(&section.anchor),
            title: Some(&section.title),
            parent: section.parent.and_then(|p| section_chunks.get(p).copied()),
            strength: &section.strength,
            symbol: None,
        });
        for (step, number) in section.steps.iter().zip(numbers) {
            chunks.push(ChunkRecord {
                kind: "step",
                start_line: step.start_line,
                end_line: step.end_line,
                text: &step.text,
                number: Some(number),
                anchor: None,
                title: None,
                parent: Some(index),
                strength: &step.strength,
                symbol: None,
            });
        }
    }
    chunks
}

/// Reads a walked file, or `None` when it was deleted after the walk.
fn read(root: &Path, path: &str) -> Result<Option<Vec<u8>>> {
    match std::fs::read(root.join(path)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::ReadFile {
            path: root.join(path),
            source,
        }),
    }
}

impl fmt::Display for IndexSummary {
    /// The summary `codecite index` prints, one fact per line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "documents: {} written, {} unchanged, {} removed",
            self.written, self.unchanged, self.removed
        )?;
        writeln!(f, "chunks: {} written", self.chunks)?;
        if self.downloaded + self.cached > 0 {
            writeln!(
                f,
                "fetched: {} downloaded, {} from the cache",
                self.downloaded, self.cached
            )?;
        }
        if !self.skipped.is_empty() {
            let parts: Vec<String> = self
                .skipped
                .iter()
                .map(|(reason, count)| format!("{count} {}", reason.label()))
                .collect();
            writeln!(f, "skipped: {}", parts.join(", "))?;
        }
        for (name, reason) in &self.skipped_sources {
            writeln!(f, "source {name} not indexed: {reason}")?;
        }
        for name in &self.removed_sources {
            writeln!(f, "source {name} removed: the run no longer indexes it")?;
        }
        for name in &self.unpinned_guidance {
            writeln!(
                f,
                "guidance {name} is not pinned: give it a sha256 so a changed rule stops the run until it is reviewed"
            )?;
        }
        write!(f, "elapsed: {:.2}s", self.elapsed.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_repo::{Remote, Repo};
    use crate::test_server::Server;
    use rusqlite::Connection;
    use std::path::PathBuf;

    /// A spec chunk as (kind, number, anchor, parent's number, strength).
    type SpecRow = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );

    fn local(repo: &Repo, name: Option<&str>) -> Source {
        Source::Local(LocalSource {
            name: name.map(str::to_owned),
            path: repo.path().to_owned(),
            license: Some("MIT".into()),
            license_class: LicenseClass::Adapt,
            include: Vec::new(),
            exclude: Vec::new(),
        })
    }

    fn document(url: String, sha256: &str) -> Source {
        Source::Document(DocumentSource {
            name: "rfc9449".into(),
            url,
            sha256: sha256.into(),
            license: Some("IETF Trust".into()),
            license_class: LicenseClass::Quote,
            current: true,
            aliases: Vec::new(),
            obsoleted_by: None,
            updated_by: None,
        })
    }

    /// An RFC page with sections 1, 4, 4.3 (two steps), 4.3.1, A and A.1.
    const SPEC: &str = include_str!("../tests/fixtures/spec/rfc-v3.html");

    fn sha256(text: &str) -> String {
        crate::fetch::hex(&Sha256::digest(text))
    }

    /// An older version of [`SPEC`], served at `/old.html`, and the
    /// manifest source pinning it, not current.
    fn old_version(server: &Server) -> Source {
        let old = SPEC.replace("is a JWT", "is a signed JWT");
        server.set("/old.html", old.as_str());
        let mut source = document(server.url("/old.html"), &sha256(&old));
        if let Source::Document(d) = &mut source {
            d.current = false;
            d.obsoleted_by = Some("rfc9449bis".into());
        }
        source
    }

    fn git(remote: &Remote, commit: &str) -> Source {
        Source::Git(GitSource {
            name: None,
            url: remote.url(),
            commit: commit.into(),
            license: Some("Apache-2.0".into()),
            license_class: LicenseClass::Adapt,
            include: Vec::new(),
            exclude: Vec::new(),
        })
    }

    /// Each document's path and `dirty` flag, by path.
    fn dirty(corpus: &Corpus) -> Vec<(String, bool)> {
        let conn = corpus.sql();
        let mut stmt = conn
            .prepare("SELECT path, dirty FROM document ORDER BY path")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn manifest(sources: Vec<Source>) -> Manifest {
        Manifest {
            name: "test".into(),
            max_age_days: 30,
            sources,
        }
    }

    /// A checkout with a Rust file, a Markdown file and a text file.
    fn fixture() -> Repo {
        let repo = Repo::new();
        repo.write(
            "src/lib.rs",
            "pub fn validate_token() {}\n\npub struct Client;\n",
        );
        repo.write("README.md", "# App\n\nValidates the token.\n");
        repo.write("notes.txt", "refresh sessions daily\n");
        repo.commit_all(&["src/lib.rs", "README.md", "notes.txt"]);
        repo
    }

    /// The corpus file for one test, deleted with the directory.
    struct Corpus {
        dir: tempfile::TempDir,
    }

    impl Corpus {
        fn new() -> Self {
            Corpus {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn path(&self) -> PathBuf {
            self.dir.path().join("corpus.db")
        }

        fn index(&self, manifest: &Manifest) -> IndexSummary {
            self.try_index(manifest).unwrap()
        }

        fn try_index(&self, manifest: &Manifest) -> Result<IndexSummary> {
            let mut store = Store::open_read_write(&self.path()).unwrap();
            let fetcher = Fetcher::new(self.dir.path().join("cache"));
            let clones = Clones::new(self.dir.path().join("repos"));
            index_corpus(
                &mut store,
                manifest,
                &WalkOptions::default(),
                &fetcher,
                &clones,
            )
        }

        fn sql(&self) -> Connection {
            Connection::open(self.path()).unwrap()
        }

        /// Each document's chunk IDs, by path.
        fn chunk_ids(&self) -> BTreeMap<String, Vec<i64>> {
            let conn = self.sql();
            let mut stmt = conn
                .prepare(
                    "SELECT d.path, c.id FROM chunk c JOIN document d ON d.id = c.document_id
                     ORDER BY c.id",
                )
                .unwrap();
            let mut ids: BTreeMap<String, Vec<i64>> = BTreeMap::new();
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap();
            for row in rows {
                let (path, id) = row.unwrap();
                ids.entry(path).or_default().push(id);
            }
            ids
        }

        /// Everything a reindex must leave as a fresh index would, without
        /// the row IDs, which record when a row was written.
        fn contents(&self) -> Contents {
            let conn = self.sql();
            let rows = |sql: &str| -> Vec<Vec<rusqlite::types::Value>> {
                let mut stmt = conn.prepare(sql).unwrap();
                let width = stmt.column_count();
                stmt.query_map([], |row| (0..width).map(|i| row.get(i)).collect())
                    .unwrap()
                    .collect::<rusqlite::Result<_>>()
                    .unwrap()
            };
            Contents {
                documents: rows(
                    "SELECT path, language, hex(content_hash), size_bytes, dirty, text
                     FROM document ORDER BY path",
                ),
                chunks: rows(
                    "SELECT d.path, c.start_line, c.end_line, c.kind, c.symbol_id, c.number,
                            c.anchor, c.title, c.strength, c.text, c.terms
                     FROM chunk c JOIN document d ON d.id = c.document_id
                     ORDER BY d.path, c.start_line, c.end_line, c.kind",
                ),
                symbols: rows(
                    "SELECT s.id, s.display_name, s.kind, d.path, s.start_line, s.end_line
                     FROM symbol s LEFT JOIN document d ON d.id = s.document_id ORDER BY s.id",
                ),
                fts: rows(
                    "SELECT (SELECT count(*) FROM chunk_fts_docsize),
                            (SELECT count(*) FROM spec_fts_docsize),
                            (SELECT block FROM chunk_fts_data WHERE id = 1),
                            (SELECT block FROM spec_fts_data WHERE id = 1)",
                ),
            }
        }

        /// The citations and scores `search` ranks for `text`.
        fn search(&self, text: &str) -> Vec<(String, f64)> {
            let store = Store::open_read_only(&self.path()).unwrap();
            let query = codecite_core::SearchQuery {
                text,
                language: None,
                path: None,
                only: None,
                limit: 20,
                allow_stale: true,
            };
            store
                .search(&query)
                .unwrap()
                .into_iter()
                .map(|r| (r.citation, r.score))
                .collect()
        }

        fn matches(&self, query: &str) -> i64 {
            self.sql()
                .query_row(
                    "SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH ?1",
                    [query],
                    |row| row.get(0),
                )
                .unwrap()
        }
    }

    /// A store's rows as [`Corpus::contents`] reads them. Deriving
    /// `PartialEq` and `Debug` lets one `assert_eq!` compare two whole
    /// stores and print where they differ.
    #[derive(PartialEq, Debug)]
    struct Contents {
        documents: Vec<Vec<rusqlite::types::Value>>,
        chunks: Vec<Vec<rusqlite::types::Value>>,
        symbols: Vec<Vec<rusqlite::types::Value>>,
        /// Row counts and the averages records BM25 reads, for `chunk_fts`
        /// and `spec_fts`.
        fts: Vec<Vec<rusqlite::types::Value>>,
    }

    /// A guidance document: a title, a scoped rule with a subsection, a
    /// rule for one dependency, and a rule for every file.
    const RULES: &str = "# House rules\n\n## 1 Settings\n\n<!-- codecite-scope languages = [\"python\"] -->\n\
        Code MUST read settings through `get_settings`.\n\n### 1.1 No environment\n\n\
        Handlers MUST NOT read `os.environ`.\n\n## 2 Partner calls\n\n\
        Calls SHOULD go through `partner_client`.\n\n## 3 Headers\n\nEvery file MUST carry the license header.\n";

    fn guidance(
        path: Option<PathBuf>,
        url: Option<String>,
        sha256: Option<String>,
    ) -> GuidanceSource {
        GuidanceSource {
            name: "house-rules".into(),
            path,
            url,
            sha256,
            license: None,
            license_class: LicenseClass::Quote,
            aliases: vec!["House Rules".into()],
            scope: RuleScope::default(),
            rules: BTreeMap::new(),
        }
    }

    /// Each stored rule scope as (number, kind, value), sorted.
    fn scopes(corpus: &Corpus) -> Vec<(String, String, String)> {
        let conn = corpus.sql();
        let mut stmt = conn
            .prepare("SELECT number, kind, value FROM rule_scope ORDER BY number, kind, value")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn scope(number: &str, kind: &str, value: &str) -> (String, String, String) {
        (number.into(), kind.into(), value.into())
    }

    #[test]
    fn a_guidance_file_is_indexed_as_rules_with_scopes() {
        let corpus = Corpus::new();
        let file = corpus.dir.path().join("RULES.md");
        std::fs::write(&file, RULES).unwrap();
        let mut rules = guidance(Some(file.clone()), None, None);
        rules.scope.paths = vec!["app/**".into()];
        rules.rules.insert(
            "2".into(),
            RuleScope {
                imports: vec!["httpx".into()],
                ..RuleScope::default()
            },
        );
        let summary = corpus.index(&manifest(vec![Source::Guidance(rules.clone())]));
        assert_eq!((summary.written, summary.chunks), (1, 4));
        assert_eq!(summary.unpinned_guidance, ["house-rules"]);
        assert!(
            summary
                .to_string()
                .contains("guidance house-rules is not pinned"),
            "{summary}"
        );

        // §1's comment, inherited by §1.1, the manifest's rule for §2 and
        // the default for §3.
        assert_eq!(
            scopes(&corpus),
            [
                scope("1", "language", "python"),
                scope("1.1", "language", "python"),
                scope("2", "import", "httpx"),
                scope("3", "path", "app/**"),
            ]
        );
        let (kind, guidance, language): (String, bool, Option<String>) = corpus
            .sql()
            .query_row(
                "SELECT s.kind, s.guidance, d.language FROM source s JOIN document d ON d.source_id = s.id",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (kind.as_str(), guidance, language.as_deref()),
            ("document", true, Some("markdown"))
        );

        // `read` resolves a rule's citation and the alias, as for a spec.
        let store = Store::open_read_only(&corpus.path()).unwrap();
        let read = store
            .read("House Rules §1.1", &codecite_core::ReadOptions::default())
            .unwrap();
        let hash = &sha256(RULES)[..12];
        assert_eq!(
            read.citation,
            format!("house-rules@sha256:{hash}#section-1.1")
        );
        assert_eq!(
            read.text,
            "### 1.1 No environment\n\nHandlers MUST NOT read `os.environ`."
        );
        assert_eq!(read.strength, Some(vec!["MUST NOT".to_owned()]));

        // `context` pushes the rules for a Python handler that imports httpx.
        let target = codecite_core::ContextTarget::describe(
            "app/handlers/partner.py".into(),
            Some("import httpx\n"),
            None,
        );
        let context = store
            .context(&codecite_core::ContextQuery {
                target: &target,
                allow_stale: false,
            })
            .unwrap();
        let cited: Vec<&str> = context
            .rules
            .iter()
            .map(|r| r.citation.rsplit_once('#').unwrap().1)
            .collect();
        assert_eq!(
            cited,
            ["section-1", "section-1.1", "section-3", "section-2"]
        );
        drop(store);

        // A changed scope in the manifest is stored without rewriting the
        // document.
        rules.rules.clear();
        let summary = corpus.index(&manifest(vec![Source::Guidance(rules)]));
        assert_eq!((summary.written, summary.unchanged), (0, 1));
        assert_eq!(scopes(&corpus)[2], scope("2", "path", "app/**"));
    }

    #[test]
    fn an_edited_guidance_file_is_a_new_version() {
        let corpus = Corpus::new();
        let file = corpus.dir.path().join("RULES.md");
        std::fs::write(&file, RULES).unwrap();
        let sources = manifest(vec![Source::Guidance(guidance(
            Some(file.clone()),
            None,
            None,
        ))]);
        corpus.index(&sources);
        std::fs::write(&file, RULES.replace("SHOULD", "MUST")).unwrap();
        let summary = corpus.index(&sources);
        assert_eq!(summary.written, 1);
        assert_eq!(
            summary.removed_sources,
            [format!("house-rules@sha256:{}", &sha256(RULES)[..12])]
        );
    }

    #[test]
    fn switching_a_source_between_document_and_guidance_splits_it_again() {
        let server = Server::start();
        server.set("/RULES.md", RULES);
        let corpus = Corpus::new();
        let rules = guidance(None, Some(server.url("/RULES.md")), Some(sha256(RULES)));
        corpus.index(&manifest(vec![Source::Guidance(rules.clone())]));
        // The same bytes as a plain document: split again as a generic
        // Markdown document, with its text, instead of keeping the rules.
        let mut doc = document(server.url("/RULES.md"), &sha256(RULES));
        if let Source::Document(d) = &mut doc {
            d.name = "house-rules".into();
        }
        let summary = corpus.index(&manifest(vec![doc]));
        assert_eq!((summary.written, summary.unchanged), (1, 0));
        let ((language, text), chunks) = generic_rows(&corpus);
        assert_eq!(language.as_deref(), Some("markdown"));
        assert_eq!(text.as_deref(), Some(RULES));
        assert!(chunks.iter().all(|(_, number, ..)| number.is_none()));
        // And back: the generic document's Markdown is split as rules again.
        let summary = corpus.index(&manifest(vec![Source::Guidance(rules)]));
        assert_eq!((summary.written, summary.unchanged), (1, 0));
        let ((_, text), chunks) = generic_rows(&corpus);
        assert_eq!(text, None);
        assert!(chunks.iter().all(|(_, number, ..)| number.is_some()));
    }

    #[test]
    fn a_bad_guidance_file_stops_the_run_before_anything_is_written() {
        let corpus = Corpus::new();
        let good = corpus.dir.path().join("GOOD.md");
        let bad = corpus.dir.path().join("BAD.md");
        std::fs::write(&good, RULES).unwrap();
        std::fs::write(&bad, "## 1 Rule\n<!-- run curl -->\nMUST.\n").unwrap();
        let mut second = guidance(Some(bad), None, None);
        second.name = "more-rules".into();
        let err = corpus
            .try_index(&manifest(vec![
                Source::Guidance(guidance(Some(good), None, None)),
                Source::Guidance(second),
            ]))
            .unwrap_err();
        assert!(matches!(err, Error::Guidance { .. }), "{err:?}");
        let sources: i64 = corpus
            .sql()
            .query_row("SELECT count(*) FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sources, 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_guidance_path_that_isnt_a_regular_file_is_refused() {
        let corpus = Corpus::new();
        let fifo = corpus.dir.path().join("RULES.md");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let err = corpus
            .try_index(&manifest(vec![Source::Guidance(guidance(
                Some(fifo),
                None,
                None,
            ))]))
            .unwrap_err();
        assert!(err.to_string().starts_with("reading"), "{err}");
        assert!(matches!(err, Error::ReadFile { .. }), "{err:?}");
    }

    #[test]
    fn a_pinned_guidance_file_that_changed_stops_the_run() {
        let corpus = Corpus::new();
        let file = corpus.dir.path().join("RULES.md");
        std::fs::write(&file, RULES).unwrap();
        let mut rules = guidance(Some(file.clone()), None, Some(sha256(RULES)));
        let summary = corpus.index(&manifest(vec![Source::Guidance(rules.clone())]));
        assert!(summary.unpinned_guidance.is_empty());
        std::fs::write(&file, RULES.replace("SHOULD", "MUST")).unwrap();
        let err = corpus
            .try_index(&manifest(vec![Source::Guidance(rules.clone())]))
            .unwrap_err();
        assert!(matches!(err, Error::GuidanceChanged { .. }), "{err:?}");
        assert!(err.to_string().contains(&sha256(RULES)), "{err}");
        rules.sha256 = None;
        assert_eq!(
            corpus
                .index(&manifest(vec![Source::Guidance(rules)]))
                .written,
            1
        );
    }

    #[test]
    fn a_fetched_guidance_document_is_pinned_by_its_hash() {
        let server = Server::start();
        server.set("/RULES.md", RULES);
        let corpus = Corpus::new();
        let rules = guidance(None, Some(server.url("/RULES.md")), Some(sha256(RULES)));
        let summary = corpus.index(&manifest(vec![Source::Guidance(rules.clone())]));
        assert_eq!((summary.downloaded, summary.written), (1, 1));

        let wrong = guidance(None, Some(server.url("/RULES.md")), Some(sha256("other")));
        let err = Corpus::new()
            .try_index(&manifest(vec![Source::Guidance(wrong)]))
            .unwrap_err();
        assert!(matches!(err, Error::Fetch(_)), "{err:?}");
    }

    #[test]
    fn guidance_errors_name_the_source_and_the_rule() {
        let corpus = Corpus::new();
        let file = corpus.dir.path().join("RULES.md");
        std::fs::write(&file, RULES).unwrap();

        let mut rules = guidance(Some(file.clone()), None, None);
        rules.rules.insert("9".into(), RuleScope::default());
        let err = corpus
            .try_index(&manifest(vec![Source::Guidance(rules)]))
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "guidance `house-rules`: `rules.\"9\"` names no rule in the document"
        );

        std::fs::write(
            &file,
            "## 1 Rule\n<!-- codecite-scope paths = [\"a/[\"] -->\nMUST.\n",
        )
        .unwrap();
        let err = corpus
            .try_index(&manifest(vec![Source::Guidance(guidance(
                Some(file.clone()),
                None,
                None,
            ))]))
            .unwrap_err();
        assert!(matches!(err, Error::Guidance { .. }), "{err:?}");

        std::fs::write(&file, RULES).unwrap();
        let mut named = guidance(Some(file), None, None);
        named.name = "app".into();
        let repo = fixture();
        let err = corpus
            .try_index(&manifest(vec![
                local(&repo, Some("app")),
                Source::Guidance(named),
            ]))
            .unwrap_err();
        assert!(matches!(err, Error::DuplicateSource { .. }), "{err:?}");

        let missing = guidance(Some(corpus.dir.path().join("nope.md")), None, None);
        let err = corpus
            .try_index(&manifest(vec![Source::Guidance(missing)]))
            .unwrap_err();
        assert!(matches!(err, Error::ReadFile { .. }), "{err:?}");
    }

    #[test]
    fn markdown_sections_store_their_title_and_anchor() {
        let repo = Repo::new();
        repo.write(
            "README.md",
            "# Getting Started\n\nRun it.\n\n## Install (Linux)\n\napt install app\n",
        );
        repo.commit_all(&["README.md"]);
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![local(&repo, Some("app"))]));

        let conn = corpus.sql();
        let mut stmt = conn
            .prepare("SELECT start_line, end_line, title, anchor FROM chunk ORDER BY start_line")
            .unwrap();
        let rows: Vec<(u32, u32, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            [
                (1, 3, "Getting Started".into(), "getting-started".into()),
                (5, 7, "Install (Linux)".into(), "install-linux".into()),
            ]
        );
    }

    #[test]
    fn indexes_a_local_source() {
        let repo = fixture();
        let corpus = Corpus::new();
        let summary = corpus.index(&manifest(vec![local(&repo, Some("app"))]));

        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (3, 0, 0)
        );
        assert_eq!(summary.chunks, 4);
        let ids = corpus.chunk_ids();
        assert_eq!(
            ids.keys().collect::<Vec<_>>(),
            ["README.md", "notes.txt", "src/lib.rs"]
        );
        assert_eq!(corpus.matches("symbol:validate"), 1);
        assert_eq!(corpus.matches("token"), 2);
        // The file's text is kept whole, the blank line no chunk holds too.
        let text: String = corpus
            .sql()
            .query_row(
                "SELECT text FROM document WHERE path = 'src/lib.rs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(text, "pub fn validate_token() {}\n\npub struct Client;\n");
        let (name, kind, revision, class): (String, String, String, String) = corpus
            .sql()
            .query_row(
                "SELECT name, kind, revision, license_class FROM source",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (name.as_str(), kind.as_str(), class.as_str()),
            ("app", "local", "adapt")
        );
        assert_eq!(revision.len(), 40);
        let symbol: String = corpus
            .sql()
            .query_row(
                "SELECT id FROM symbol WHERE display_name = 'Client'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(symbol, "codecite . app . src/`lib.rs`/Client#");
    }

    #[test]
    fn reindexing_rewrites_only_the_changed_document() {
        let repo = fixture();
        let corpus = Corpus::new();
        let manifest = manifest(vec![local(&repo, Some("app"))]);
        corpus.index(&manifest);
        let before = corpus.chunk_ids();

        repo.write("src/lib.rs", "pub fn refresh_session() {}\n");
        let summary = corpus.index(&manifest);

        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (1, 2, 0)
        );
        assert_eq!(summary.chunks, 1);
        let after = corpus.chunk_ids();
        assert_eq!(after["README.md"], before["README.md"]);
        assert_eq!(after["notes.txt"], before["notes.txt"]);
        // SQLite may reuse the freed chunk IDs, so check the content instead.
        assert_eq!(after["src/lib.rs"].len(), 1);
        assert_eq!(corpus.matches("symbol:validate"), 0);
        assert_eq!(corpus.matches("symbol:refresh"), 1);
        assert_eq!(
            dirty(&corpus),
            [
                ("README.md".to_owned(), false),
                ("notes.txt".to_owned(), false),
                ("src/lib.rs".to_owned(), true)
            ]
        );
    }

    #[test]
    fn a_deleted_file_loses_its_chunks() {
        let repo = fixture();
        let corpus = Corpus::new();
        let manifest = manifest(vec![local(&repo, Some("app"))]);
        corpus.index(&manifest);
        assert_eq!(corpus.matches("refresh"), 1);

        std::fs::remove_file(repo.path().join("notes.txt")).unwrap();
        let summary = corpus.index(&manifest);

        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (0, 2, 1)
        );
        assert_eq!(summary.skipped[&SkipReason::Deleted], 1);
        assert!(!corpus.chunk_ids().contains_key("notes.txt"));
        assert_eq!(corpus.matches("refresh"), 0);
    }

    #[test]
    fn a_file_that_becomes_skipped_is_removed() {
        let repo = fixture();
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![local(&repo, Some("app"))]));

        repo.write("notes.txt", b"binary\0now\n");
        let summary = corpus.index(&manifest(vec![local(&repo, Some("app"))]));
        assert_eq!(summary.removed, 1);
        assert_eq!(summary.skipped[&SkipReason::Binary], 1);

        repo.write("notes.txt", b"caf\xe9\n");
        repo.write("README.md", "# App\n");
        let summary = corpus.index(&manifest(vec![local(&repo, Some("app"))]));
        assert_eq!(summary.skipped[&SkipReason::NonUtf8Content], 1);
        assert_eq!((summary.written, summary.removed), (1, 0));
        assert_eq!(
            corpus.chunk_ids().keys().collect::<Vec<_>>(),
            ["README.md", "src/lib.rs"]
        );
    }

    #[test]
    fn sources_the_run_does_not_index_are_removed() {
        let repo = fixture();
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![local(&repo, Some("app"))]));

        let mut excluded = local(&repo, Some("app"));
        if let Source::Local(l) = &mut excluded {
            l.license_class = LicenseClass::Excluded;
        }
        let summary = corpus.index(&manifest(vec![excluded]));
        assert_eq!(summary.removed_sources, ["app"]);
        assert_eq!(
            summary.skipped_sources,
            [("app".to_owned(), "use = \"excluded\"")]
        );
        assert!(corpus.chunk_ids().is_empty());
        assert_eq!(corpus.matches("token"), 0);
    }

    #[test]
    fn the_name_defaults_to_the_directory_and_must_be_unique() {
        let repo = fixture();
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![local(&repo, None)]));
        let dir = repo.path().canonicalize().unwrap();
        let name: String = corpus
            .sql()
            .query_row("SELECT name FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, dir.file_name().unwrap().to_str().unwrap());

        let twice = manifest(vec![local(&repo, Some("app")), local(&repo, Some("app"))]);
        let err = corpus.try_index(&twice).unwrap_err();
        assert!(
            matches!(&err, Error::DuplicateSource { name } if name == "app"),
            "{err:?}"
        );
    }

    #[test]
    fn indexes_a_git_source_at_its_pinned_commit() {
        let repo = fixture();
        let remote = Remote::new();
        let commit = repo.push(&remote, "main");
        // The checkout the remote was pushed from has moved on; the pin hasn't.
        repo.write("src/lib.rs", "pub fn unpushed() {}\n");
        repo.commit_all(&["src/lib.rs"]);
        let corpus = Corpus::new();
        let summary = corpus.index(&manifest(vec![git(&remote, &commit)]));

        assert_eq!((summary.written, summary.chunks), (3, 4));
        assert!(summary.skipped_sources.is_empty());
        let source: (String, String, String, String, String) = corpus
            .sql()
            .query_row(
                "SELECT name, kind, url, revision, license_spdx FROM source",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        // The name is the URL's last segment: the remote's directory name.
        let dir = remote.url().rsplit('/').next().unwrap().to_owned();
        assert_eq!(
            source,
            (dir, "git".into(), remote.url(), commit, "Apache-2.0".into())
        );
        assert_eq!(corpus.matches("symbol:validate"), 1);
        assert_eq!(corpus.matches("symbol:unpushed"), 0);
        assert!(dirty(&corpus).iter().all(|(_, dirty)| !dirty));
    }

    #[test]
    fn reindexing_a_git_source_at_a_new_commit_rewrites_only_what_changed() {
        let repo = fixture();
        let remote = Remote::new();
        let first = repo.push(&remote, "main");
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![git(&remote, &first)]));
        let before = corpus.chunk_ids();

        repo.write("src/lib.rs", "pub fn refresh_session() {}\n");
        repo.git(&["rm", "-q", "notes.txt"]);
        repo.commit_all(&["src/lib.rs"]);
        let second = repo.push(&remote, "main");
        let summary = corpus.index(&manifest(vec![git(&remote, &second)]));

        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (1, 1, 1)
        );
        let after = corpus.chunk_ids();
        assert_eq!(after["README.md"], before["README.md"]);
        assert!(!after.contains_key("notes.txt"));
        assert_eq!(corpus.matches("symbol:refresh"), 1);
        assert_eq!(corpus.matches("symbol:validate"), 0);
        let revision: String = corpus
            .sql()
            .query_row("SELECT revision FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(revision, second);
        assert!(dirty(&corpus).iter().all(|(_, dirty)| !dirty));

        // Pinning the first commit again brings its files back.
        let summary = corpus.index(&manifest(vec![git(&remote, &first)]));
        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (2, 1, 0)
        );
    }

    #[test]
    fn reindexing_at_a_new_commit_matches_a_fresh_index() {
        let repo = fixture();
        repo.write(
            "src/session.rs",
            "pub fn open_session() {}\n\npub fn refresh_session() {}\n\n\
             pub fn close_session() {}\n\npub fn expire_session() {}\n",
        );
        repo.write(
            "docs/guide.md",
            "# Guide\n\n## Tokens\n\nValidate each token.\n\n## Sessions\n\n\
             Refresh the session hourly.\n\n## Logging\n\nLog every refresh.\n",
        );
        repo.commit_all(&["src/session.rs", "docs/guide.md"]);
        let remote = Remote::new();
        let first = repo.push(&remote, "main");

        repo.write(
            "src/lib.rs",
            "pub fn validate_token() {}\n\npub fn refresh_token() {}\n",
        );
        repo.git(&["rm", "-q", "notes.txt"]);
        repo.write(
            "src/session.rs",
            "pub fn open_session() {}\n\npub fn refresh_session() {}\n\n\
             pub fn expire_session() {}\n",
        );
        repo.write("src/audit.rs", "pub fn log_refresh() {}\n");
        repo.commit_all(&["src/lib.rs", "src/session.rs", "src/audit.rs"]);
        let second = repo.push(&remote, "main");

        let reindexed = Corpus::new();
        reindexed.index(&manifest(vec![git(&remote, &first)]));
        reindexed.index(&manifest(vec![git(&remote, &second)]));
        let fresh = Corpus::new();
        fresh.index(&manifest(vec![git(&remote, &second)]));

        assert_eq!(reindexed.contents(), fresh.contents());
        for text in ["validate token", "refresh session", "hourly", "log refresh"] {
            assert_eq!(reindexed.search(text), fresh.search(text), "{text}");
        }
    }

    #[test]
    fn equal_scores_rank_in_citation_order_whatever_was_indexed_first() {
        let repo = Repo::new();
        repo.write("b.rs", "pub fn tied() {}\n");
        repo.commit_all(&["b.rs"]);
        let reindexed = Corpus::new();
        let manifest = manifest(vec![local(&repo, Some("app"))]);
        reindexed.index(&manifest);
        // `a.rs` is written second, so its chunk has the higher ID.
        repo.write("a.rs", "pub fn tied() {}\n");
        repo.commit_all(&["a.rs"]);
        reindexed.index(&manifest);
        let fresh = Corpus::new();
        fresh.index(&manifest);

        let results = reindexed.search("tied");
        let paths: Vec<_> = results
            .iter()
            .map(|(c, _)| c.split_once(':').unwrap().1)
            .collect();
        assert_eq!(paths, ["a.rs#L1", "b.rs#L1"]);
        assert_eq!(results[0].1, results[1].1);
        assert_eq!(results, fresh.search("tied"));
    }

    #[test]
    fn a_missing_commit_stops_the_run_before_any_write() {
        let repo = fixture();
        let remote = Remote::new();
        repo.push(&remote, "main");
        let corpus = Corpus::new();
        let missing = "0123456789abcdef0123456789abcdef01234567";
        let manifest = manifest(vec![local(&repo, Some("app")), git(&remote, missing)]);

        let err = corpus.try_index(&manifest).unwrap_err();
        assert!(
            matches!(&err, Error::Clone(crate::CloneError::CommitNotFound { commit, .. })
                if commit == missing),
            "{err:?}"
        );
        let sources: i64 = corpus
            .sql()
            .query_row("SELECT count(*) FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sources, 0);
    }

    #[test]
    fn git_sources_are_named_by_url_and_excluded_ones_are_not_cloned() {
        let remote = Remote::new();
        let mut source = git(&remote, "0123456789abcdef0123456789abcdef01234567");
        let Source::Git(g) = &mut source else {
            unreachable!()
        };
        for (url, name) in [
            (
                "https://github.com/acme/identity-model.git",
                "identity-model",
            ),
            ("https://github.com/acme/identity-model/", "identity-model"),
            ("git@github.com:acme/tools.git", "tools"),
            ("file:///srv/git/app", "app"),
            ("/", "git"),
        ] {
            g.url = url.into();
            assert_eq!(git_name(g), name, "{url}");
        }
        g.name = Some("client".into());
        assert_eq!(git_name(g), "client");

        g.license_class = LicenseClass::Excluded;
        let corpus = Corpus::new();
        let summary = corpus.index(&manifest(vec![source]));
        assert_eq!(
            summary.skipped_sources,
            [("client".to_owned(), "use = \"excluded\"")]
        );
        assert!(!corpus.dir.path().join("repos").exists());
    }

    #[test]
    fn a_document_is_indexed_as_sections_and_steps() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let corpus = Corpus::new();
        let manifest = manifest(vec![document(server.url("/rfc9449.html"), &sha256(SPEC))]);

        let summary = corpus.index(&manifest);
        assert_eq!((summary.downloaded, summary.written), (1, 1));
        // Six sections, and two steps in §4.3.
        assert_eq!(summary.chunks, 8);
        assert!(summary.skipped_sources.is_empty());
        let source: (String, String, String, bool, String, Option<String>) = corpus
            .sql()
            .query_row(
                "SELECT name, kind, revision, is_current, license_class, license_spdx FROM source",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            source,
            (
                "rfc9449".to_owned(),
                "document".to_owned(),
                sha256(SPEC),
                true,
                "quote".to_owned(),
                Some("IETF Trust".to_owned())
            )
        );

        let chunks: Vec<SpecRow> = {
            let conn = corpus.sql();
            let mut stmt = conn
                .prepare(
                    "SELECT c.kind, c.number, c.anchor, p.number, c.strength
                     FROM chunk c LEFT JOIN chunk p ON p.id = c.parent_id ORDER BY c.id",
                )
                .unwrap();
            stmt.query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
        };
        let row = |kind: &str,
                   number: &str,
                   anchor: Option<&str>,
                   parent: Option<&str>,
                   strength: Option<&str>| {
            (
                kind.to_owned(),
                number.to_owned(),
                anchor.map(str::to_owned),
                parent.map(str::to_owned),
                strength.map(str::to_owned),
            )
        };
        assert_eq!(
            chunks,
            [
                row("section", "1", Some("section-1"), None, None),
                row("section", "4", Some("section-4"), None, None),
                row(
                    "section",
                    "4.3",
                    Some("section-4.3"),
                    Some("4"),
                    Some("MUST")
                ),
                row("step", "1", None, Some("4.3"), None),
                row("step", "2", None, Some("4.3"), None),
                row(
                    "section",
                    "4.3.1",
                    Some("section-4.3.1"),
                    Some("4.3"),
                    Some("MAY")
                ),
                row("section", "A", Some("appendix-A"), None, None),
                row("section", "A.1", Some("appendix-A.1"), Some("A"), None),
            ]
        );
        // Spec text is searchable like code: §4.3 and its second step.
        assert_eq!(corpus.matches("jose"), 2);
        assert_eq!(corpus.matches("path:rfc9449"), 8);
    }

    #[test]
    fn reindexing_an_unchanged_pin_is_a_no_op() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let corpus = Corpus::new();
        let manifest = manifest(vec![document(server.url("/rfc9449.html"), &sha256(SPEC))]);
        corpus.index(&manifest);
        let before = corpus.chunk_ids();

        let second = corpus.index(&manifest);
        assert_eq!((second.downloaded, second.cached), (0, 1));
        assert_eq!((second.written, second.unchanged, second.chunks), (0, 1, 0));
        assert!(second.removed_sources.is_empty());
        assert_eq!(corpus.chunk_ids(), before);
        assert_eq!(server.requests(), 1);
    }

    #[test]
    fn two_versions_of_one_document_are_both_indexed() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let corpus = Corpus::new();
        let current = document(server.url("/rfc9449.html"), &sha256(SPEC));
        let old = old_version(&server);
        let summary = corpus.index(&manifest(vec![current.clone(), old]));
        assert_eq!((summary.written, summary.chunks), (2, 16));

        let versions: Vec<(String, bool, Option<String>)> = {
            let conn = corpus.sql();
            let mut stmt = conn
                .prepare(
                    "SELECT revision, is_current, obsoleted_by FROM source
                     WHERE name = 'rfc9449' ORDER BY is_current",
                )
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        let old_sha = sha256(&SPEC.replace("is a JWT", "is a signed JWT"));
        assert_eq!(
            versions,
            [
                (old_sha.clone(), false, Some("rfc9449bis".to_owned())),
                (sha256(SPEC), true, None)
            ]
        );
        assert_eq!(corpus.matches("signed"), 1);

        // Unpinning the old version deletes it and everything stored for it.
        let summary = corpus.index(&manifest(vec![current]));
        assert_eq!(
            summary.removed_sources,
            [format!("rfc9449@sha256:{}", &old_sha[..12])]
        );
        assert_eq!(corpus.matches("signed"), 0);
        assert_eq!(corpus.matches("jose"), 2);
    }

    /// A page in no spec layout, with headings `Release notes`, `v2`
    /// and `v1`, a nav bar and a script.
    const PLAIN: &str = include_str!("../tests/fixtures/document/plain.html");

    /// The stored document's (language, text), and its chunks as (kind,
    /// number, anchor, title), in order.
    type GenericRows = (
        (Option<String>, Option<String>),
        Vec<(String, Option<String>, Option<String>, Option<String>)>,
    );

    fn generic_rows(corpus: &Corpus) -> GenericRows {
        let conn = corpus.sql();
        let document = conn
            .query_row("SELECT language, text FROM document", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        let mut stmt = conn
            .prepare("SELECT kind, number, anchor, title FROM chunk ORDER BY id")
            .unwrap();
        let chunks = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        (document, chunks)
    }

    #[test]
    fn a_page_that_is_not_a_spec_is_indexed_as_a_document() {
        let server = Server::start();
        server.set("/notes.html", PLAIN);
        let corpus = Corpus::new();
        let manifest = manifest(vec![document(server.url("/notes.html"), &sha256(PLAIN))]);

        let summary = corpus.index(&manifest);
        assert_eq!((summary.downloaded, summary.written), (1, 1));
        let ((language, text), chunks) = generic_rows(&corpus);
        assert_eq!(language.as_deref(), Some("html"));
        let text = text.expect("a generic document stores its text");
        assert!(text.starts_with("Release notes\n"), "{text}");
        assert!(!text.contains("Home"), "{text}");
        let sections: Vec<(Option<String>, Option<String>)> = chunks
            .iter()
            .filter(|(kind, ..)| kind == "section")
            .map(|(_, _, anchor, title)| (anchor.clone(), title.clone()))
            .collect();
        assert_eq!(
            sections,
            [
                (Some("release-notes".into()), Some("Release notes".into())),
                (Some("v2".into()), Some("Version 2".into())),
                (Some("v1".into()), Some("Version 1".into())),
            ]
        );
        assert!(
            chunks
                .iter()
                .all(|(kind, number, ..)| (kind == "section" || kind == "window")
                    && number.is_none()),
            "{chunks:?}"
        );
        // Only spec chunks, which have numbers, are in the stemmed index.
        let spec_rows: i64 = corpus
            .sql()
            .query_row("SELECT count(*) FROM spec_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(spec_rows, 0);

        // The same pin again is stored already.
        let again = corpus.index(&manifest);
        assert_eq!((again.unchanged, again.written), (1, 0));
    }

    #[test]
    fn a_markdown_page_is_indexed_as_markdown() {
        let server = Server::start();
        let page = "# Notes\n\nSome text.\n\n## Usage\n\nRun it.\n";
        server.set("/notes.md", page);
        let corpus = Corpus::new();
        let manifest = manifest(vec![document(server.url("/notes.md"), &sha256(page))]);

        corpus.index(&manifest);
        let ((language, text), chunks) = generic_rows(&corpus);
        assert_eq!(language.as_deref(), Some("markdown"));
        assert_eq!(text.as_deref(), Some(page));
        let anchors: Vec<Option<String>> = chunks
            .iter()
            .filter(|(kind, ..)| kind == "section")
            .map(|(_, _, anchor, _)| anchor.clone())
            .collect();
        assert_eq!(anchors, [Some("notes".into()), Some("usage".into())]);
    }

    #[test]
    fn a_local_source_may_not_share_a_document_name() {
        let repo = fixture();
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let corpus = Corpus::new();
        let manifest = manifest(vec![
            local(&repo, Some("rfc9449")),
            document(server.url("/rfc9449.html"), &sha256(SPEC)),
        ]);
        let err = corpus.try_index(&manifest).unwrap_err();
        assert!(
            matches!(&err, Error::DuplicateSource { name } if name == "rfc9449"),
            "{err:?}"
        );
    }

    #[test]
    fn a_changed_document_stops_the_run_before_any_write() {
        let repo = fixture();
        let server = Server::start();
        server.set("/rfc9449.html", "<h1>RFC 9449, edited</h1>\n");
        let corpus = Corpus::new();
        let manifest = manifest(vec![
            local(&repo, Some("app")),
            document(server.url("/rfc9449.html"), &sha256(SPEC)),
        ]);

        let err = corpus.try_index(&manifest).unwrap_err();
        assert!(
            matches!(&err, Error::Fetch(crate::FetchError::Mismatch { name, .. }) if name == "rfc9449"),
            "{err:?}"
        );
        let sources: i64 = corpus
            .sql()
            .query_row("SELECT count(*) FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sources, 0);
    }

    #[test]
    fn aliases_are_stored_and_name_one_document() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let corpus = Corpus::new();
        let mut doc = document(server.url("/rfc9449.html"), &sha256(SPEC));
        if let Source::Document(d) = &mut doc {
            d.aliases = vec!["DPoP".into(), "OAuth  DPoP".into()];
        }
        corpus.index(&manifest(vec![doc.clone()]));
        let store = Store::open_read_only(&corpus.path()).unwrap();
        let aliases = store.aliases().unwrap();
        assert_eq!(aliases.get("dpop"), Some("rfc9449"));
        assert_eq!(aliases.get("OAuth DPoP"), Some("rfc9449"));
        drop(store);

        let mut other = doc.clone();
        if let Source::Document(d) = &mut other {
            d.name = "rfc9450".into();
            d.aliases = vec!["dpop".into()];
        }
        let err = corpus.try_index(&manifest(vec![doc, other])).unwrap_err();
        assert!(
            matches!(&err, Error::DuplicateAlias { alias, first, second }
                if alias == "dpop" && first == "rfc9449" && second == "rfc9450"),
            "{err:?}"
        );
    }

    #[test]
    fn excluded_documents_are_not_fetched() {
        let server = Server::start();
        let corpus = Corpus::new();
        let mut doc = document(server.url("/rfc9449.html"), &sha256(SPEC));
        if let Source::Document(d) = &mut doc {
            d.license_class = LicenseClass::Excluded;
        }
        let summary = corpus.index(&manifest(vec![doc]));
        assert_eq!(server.requests(), 0);
        assert_eq!(
            summary.skipped_sources,
            [("rfc9449".to_owned(), "use = \"excluded\"")]
        );
    }

    #[test]
    fn summary_prints_each_count() {
        let summary = IndexSummary {
            written: 2,
            unchanged: 5,
            removed: 1,
            chunks: 9,
            downloaded: 1,
            cached: 2,
            skipped: BTreeMap::from([(SkipReason::Ignored, 3), (SkipReason::Binary, 1)]),
            skipped_sources: vec![("client".into(), "use = \"excluded\"")],
            removed_sources: vec!["old".into()],
            unpinned_guidance: vec!["rules".into()],
            elapsed: Duration::from_millis(1250),
        };
        assert_eq!(
            summary.to_string(),
            "documents: 2 written, 5 unchanged, 1 removed\n\
             chunks: 9 written\n\
             fetched: 1 downloaded, 2 from the cache\n\
             skipped: 3 ignored, 1 binary\n\
             source client not indexed: use = \"excluded\"\n\
             source old removed: the run no longer indexes it\n\
             guidance rules is not pinned: give it a sha256 so a changed rule stops the run until it is reviewed\n\
             elapsed: 1.25s"
        );
    }

    /// Back-Channel Logout 1.0 with errata set 1, current, and as
    /// published before it, each cut to §2.3–§2.8 of the pinned page.
    const BACKCHANNEL: &str = include_str!("../tests/fixtures/spec/backchannel-1_0.html");
    const BACKCHANNEL_FINAL: &str =
        include_str!("../tests/fixtures/spec/backchannel-1_0-final.html");

    /// Indexes both Back-Channel Logout versions and diffs `section`.
    fn backchannel_diff(section: &str) -> codecite_core::DiffResult {
        let server = Server::start();
        server.set("/current.html", BACKCHANNEL);
        server.set("/final.html", BACKCHANNEL_FINAL);
        let version = |path: &str, text: &str, current: bool| {
            let mut source = document(server.url(path), &sha256(text));
            if let Source::Document(d) = &mut source {
                d.name = "openid-connect-backchannel-1_0".into();
                d.current = current;
            }
            source
        };
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![
            version("/current.html", BACKCHANNEL, true),
            version("/final.html", BACKCHANNEL_FINAL, false),
        ]));
        let store = Store::open_read_only(&corpus.path()).unwrap();
        store
            .diff(
                "openid-connect-backchannel-1_0",
                &codecite_core::DiffOptions {
                    section: Some(section),
                    allow_stale: true,
                    ..Default::default()
                },
            )
            .unwrap()
    }

    #[test]
    fn backchannel_logout_errata_makes_exp_required() {
        let d = backchannel_diff("2.4");
        let (old, new) = (
            format!(
                "openid-connect-backchannel-1_0@sha256:{}",
                &sha256(BACKCHANNEL_FINAL)[..12]
            ),
            format!(
                "openid-connect-backchannel-1_0@sha256:{}",
                &sha256(BACKCHANNEL)[..12]
            ),
        );
        assert_eq!((&d.from, &d.to), (&old, &new));
        assert_eq!(d.changes.len(), 1, "{:#?}", d.changes);
        let exp = &d.changes[0];
        assert_eq!(exp.change, "added");
        assert!(
            exp.to_text
                .as_deref()
                .unwrap()
                .starts_with("exp\nREQUIRED. Expiration time"),
            "{exp:?}"
        );
        assert_eq!(
            exp.to_strength.as_deref(),
            Some(&["REQUIRED".to_owned()][..])
        );
        assert_eq!(exp.from.as_deref(), Some(&*format!("{old}#section-2.4")));
        assert_eq!(exp.to.as_deref(), Some(&*format!("{new}#section-2.4")));
        // Every other claim changed only the date of its Core reference.
        assert!(d.reworded.len() >= 5, "{:#?}", d.reworded);
    }

    #[test]
    fn backchannel_logout_response_has_no_normative_change() {
        // §2.8's one change between the versions is "occured" → "occurred".
        let d = backchannel_diff("2.8");
        assert!(d.changes.is_empty(), "{:#?}", d.changes);
        assert_eq!(d.total, 0);
        assert_eq!(d.reworded.len(), 1, "{:#?}", d.reworded);
    }

    #[test]
    fn backchannel_logout_validation_adds_exp_to_a_step() {
        let d = backchannel_diff("2.6");
        assert_eq!(d.changes.len(), 1, "{:#?}", d.changes);
        let step = &d.changes[0];
        assert_eq!(step.change, "changed");
        assert!(step.to.as_deref().unwrap().ends_with("#section-2.6/step-4"));
        assert!(
            step.to_text
                .as_deref()
                .unwrap()
                .contains("iat, and exp Claims")
        );
    }
}
