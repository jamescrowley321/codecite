//! Document sources: fetch each one by URL, check its bytes against the
//! manifest's pinned sha256, and keep them in a cache named by that hash, so
//! a pinned document is downloaded once and indexing then works offline.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::DocumentSource;

/// The largest document a fetch accepts: 16 MiB. The largest RFC in HTML is
/// a few MiB; anything bigger is not a spec.
pub const MAX_DOCUMENT_SIZE: u64 = 16 * 1024 * 1024;

/// How long one download may take, from connecting to the last byte.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Fetches document sources over HTTP(S) and caches them by content hash.
///
/// The cache holds one file per document, named by the lowercase hex sha256
/// of its bytes. Since the name is the hash, a cached file can't go stale: a
/// pinned document is read from the cache whenever its hash is there.
#[derive(Debug)]
pub struct Fetcher {
    cache_dir: PathBuf,
    agent: ureq::Agent,
}

/// A document's bytes and where they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The document's bytes, exactly as served.
    pub bytes: Vec<u8>,
    /// Whether the bytes came from the cache rather than the network.
    pub from_cache: bool,
}

/// Why a document could not be fetched or did not match its pin.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// The fetched bytes do not hash to the manifest's `sha256`.
    #[error(
        "document `{name}` from {url} has sha256 {actual}, but the manifest pins {expected}; \
         check what changed, then update `sha256` (`codecite index <corpus> --print-hashes` prints it)"
    )]
    Mismatch {
        /// The document's name.
        name: String,
        /// Where it was fetched from.
        url: String,
        /// The hash the manifest pins.
        expected: String,
        /// The hash of the fetched bytes.
        actual: String,
    },

    /// The request failed, the server answered with an error status, or the
    /// connection broke while reading the body.
    #[error("fetching document `{name}` from {url}")]
    Download {
        /// The document's name.
        name: String,
        /// Where it was fetched from.
        url: String,
        /// What went wrong.
        source: io::Error,
    },

    /// The document is larger than [`MAX_DOCUMENT_SIZE`].
    #[error("document `{name}` from {url} is larger than {limit} bytes")]
    TooLarge {
        /// The document's name.
        name: String,
        /// Where it was fetched from.
        url: String,
        /// The size limit in bytes.
        limit: u64,
    },

    /// Reading or writing the document cache failed.
    #[error("document cache {}", path.display())]
    Cache {
        /// The cache file or directory.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
}

impl Fetcher {
    /// A fetcher that caches documents in `cache_dir`, created on first use.
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(concat!("codecite/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Fetcher {
            cache_dir: cache_dir.into(),
            agent,
        }
    }

    /// The document's pinned bytes: from the cache when they are there, else
    /// downloaded and checked against `doc.sha256`. A mismatch is an error
    /// and nothing is cached.
    pub fn fetch_pinned(&self, doc: &DocumentSource) -> Result<Fetched, FetchError> {
        let cached = self.cache_dir.join(&doc.sha256);
        if let Some(bytes) = self.read_cached(&cached)? {
            return Ok(Fetched {
                bytes,
                from_cache: true,
            });
        }
        let download = self.download(doc)?;
        if download.hash != doc.sha256 {
            return Err(FetchError::Mismatch {
                name: doc.name.clone(),
                url: doc.url.clone(),
                expected: doc.sha256.clone(),
                actual: download.hash,
            });
        }
        let bytes = download.keep(&self.cache_dir)?;
        Ok(Fetched {
            bytes,
            from_cache: false,
        })
    }

    /// Downloads the document now and returns the sha256 of what the URL
    /// serves, without comparing it to the pin. The bytes are cached under
    /// that hash, so pinning it later needs no second download.
    pub fn fetch_hash(&self, doc: &DocumentSource) -> Result<String, FetchError> {
        let download = self.download(doc)?;
        let hash = download.hash.clone();
        download.keep(&self.cache_dir)?;
        Ok(hash)
    }

    /// A cached document's bytes, or `None` when it isn't cached. A cached
    /// file whose bytes no longer hash to its name is deleted and treated as
    /// missing.
    fn read_cached(&self, path: &Path) -> Result<Option<Vec<u8>>, FetchError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(FetchError::Cache {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let name = path.file_name().and_then(|n| n.to_str());
        if name == Some(hex(&Sha256::digest(&bytes)).as_str()) {
            return Ok(Some(bytes));
        }
        fs::remove_file(path).map_err(|source| FetchError::Cache {
            path: path.to_owned(),
            source,
        })?;
        Ok(None)
    }

    /// Streams the response body into memory, hashing it on the way, and
    /// stops as soon as it passes [`MAX_DOCUMENT_SIZE`].
    fn download(&self, doc: &DocumentSource) -> Result<Download, FetchError> {
        let failed = |source: io::Error| FetchError::Download {
            name: doc.name.clone(),
            url: doc.url.clone(),
            source,
        };
        let response = self
            .agent
            .get(&doc.url)
            .call()
            .map_err(|e| failed(e.into_io()))?;
        // One byte past the limit tells a document of exactly the limit
        // from a larger one.
        let mut body = response
            .into_body()
            .into_reader()
            .take(MAX_DOCUMENT_SIZE + 1);

        let mut hasher = Sha256::new();
        let mut bytes = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let n = match body.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(failed(e)),
            };
            hasher.update(&buffer[..n]);
            bytes.extend_from_slice(&buffer[..n]);
        }
        if bytes.len() as u64 > MAX_DOCUMENT_SIZE {
            return Err(FetchError::TooLarge {
                name: doc.name.clone(),
                url: doc.url.clone(),
                limit: MAX_DOCUMENT_SIZE,
            });
        }
        Ok(Download {
            bytes,
            hash: hex(&hasher.finalize()),
        })
    }
}

/// A finished download: the bytes and their hash.
struct Download {
    bytes: Vec<u8>,
    hash: String,
}

impl Download {
    /// Writes the bytes into the cache under their hash and returns them.
    /// The file is written under a temporary name and renamed into place,
    /// so a crash or a concurrent run never leaves a partial cache file.
    fn keep(self, cache_dir: &Path) -> Result<Vec<u8>, FetchError> {
        let cache_error = |path: &Path| {
            let path = path.to_owned();
            move |source| FetchError::Cache { path, source }
        };
        fs::create_dir_all(cache_dir).map_err(cache_error(cache_dir))?;
        let mut file = tempfile::Builder::new()
            .prefix(".download-")
            .tempfile_in(cache_dir)
            .map_err(cache_error(cache_dir))?;
        file.write_all(&self.bytes)
            .map_err(cache_error(file.path()))?;
        let path = cache_dir.join(&self.hash);
        file.persist(&path)
            .map_err(|e| cache_error(&path)(e.error))?;
        Ok(self.bytes)
    }
}

/// Lowercase hex, the form the manifest pins.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LicenseClass;
    use crate::test_server::Server;

    const SPEC: &[u8] = b"<html><h1>RFC 9449</h1></html>\n";

    fn sha256(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    fn document(url: String, sha256: String) -> DocumentSource {
        DocumentSource {
            name: "rfc9449".into(),
            url,
            sha256,
            license: None,
            license_class: LicenseClass::Quote,
            current: true,
            aliases: Vec::new(),
            obsoleted_by: None,
            updated_by: None,
        }
    }

    #[test]
    fn hex_is_lowercase_and_two_digits_per_byte() {
        assert_eq!(hex(&[0x00, 0x0f, 0xab]), "000fab");
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_matching_document_is_fetched_and_cached() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new(cache.path().join("documents"));
        let doc = document(server.url("/rfc9449.html"), sha256(SPEC));

        let fetched = fetcher.fetch_pinned(&doc).unwrap();
        assert_eq!(fetched.bytes, SPEC);
        assert!(!fetched.from_cache);
        let cached = cache.path().join("documents").join(&doc.sha256);
        assert_eq!(fs::read(cached).unwrap(), SPEC);

        let again = fetcher.fetch_pinned(&doc).unwrap();
        assert_eq!(again.bytes, SPEC);
        assert!(again.from_cache);
        assert_eq!(server.requests(), 1);
    }

    #[test]
    fn a_mismatch_names_the_document_and_both_hashes() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new(cache.path());
        let pinned = sha256(b"the version that was pinned");
        let doc = document(server.url("/rfc9449.html"), pinned.clone());

        let err = fetcher.fetch_pinned(&doc).unwrap_err();
        let FetchError::Mismatch {
            name,
            expected,
            actual,
            ..
        } = &err
        else {
            panic!("{err:?}");
        };
        assert_eq!(
            (name.as_str(), expected, actual),
            ("rfc9449", &pinned, &sha256(SPEC))
        );
        let message = err.to_string();
        assert!(message.contains("`rfc9449`"), "{message}");
        assert!(message.contains(&pinned), "{message}");
        assert!(message.contains(&sha256(SPEC)), "{message}");
        // Nothing is cached under either hash.
        assert_eq!(fs::read_dir(cache.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_cached_document_needs_no_network() {
        let cache = tempfile::tempdir().unwrap();
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let doc = document(server.url("/rfc9449.html"), sha256(SPEC));
        Fetcher::new(cache.path()).fetch_pinned(&doc).unwrap();
        drop(server);

        // The server is gone, so a download would fail to connect.
        let fetched = Fetcher::new(cache.path()).fetch_pinned(&doc).unwrap();
        assert_eq!(fetched.bytes, SPEC);
        assert!(fetched.from_cache);

        let empty = tempfile::tempdir().unwrap();
        let err = Fetcher::new(empty.path()).fetch_pinned(&doc).unwrap_err();
        assert!(matches!(err, FetchError::Download { .. }), "{err:?}");
    }

    #[test]
    fn a_corrupt_cache_file_is_fetched_again() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let cache = tempfile::tempdir().unwrap();
        let doc = document(server.url("/rfc9449.html"), sha256(SPEC));
        fs::write(cache.path().join(&doc.sha256), b"truncated").unwrap();

        let fetched = Fetcher::new(cache.path()).fetch_pinned(&doc).unwrap();
        assert!(!fetched.from_cache);
        assert_eq!(fs::read(cache.path().join(&doc.sha256)).unwrap(), SPEC);
    }

    #[test]
    fn an_error_status_is_a_download_error() {
        let server = Server::start();
        let cache = tempfile::tempdir().unwrap();
        let doc = document(server.url("/gone.html"), sha256(SPEC));

        let err = Fetcher::new(cache.path()).fetch_pinned(&doc).unwrap_err();
        assert!(matches!(err, FetchError::Download { .. }), "{err:?}");
        let source = std::error::Error::source(&err).unwrap().to_string();
        assert!(source.contains("404"), "{source}");
    }

    #[test]
    fn a_document_past_the_size_limit_is_refused() {
        let server = Server::start();
        let size = usize::try_from(MAX_DOCUMENT_SIZE).unwrap();
        server.set("/exact.html", vec![b'a'; size]);
        server.set("/big.html", vec![b'a'; size + 1]);
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new(cache.path());

        let exact = fetcher.fetch_hash(&document(server.url("/exact.html"), String::new()));
        assert_eq!(exact.unwrap(), sha256(&vec![b'a'; size]));
        let err = fetcher
            .fetch_hash(&document(server.url("/big.html"), String::new()))
            .unwrap_err();
        assert!(matches!(err, FetchError::TooLarge { .. }), "{err:?}");
    }

    #[test]
    fn fetch_hash_reports_what_the_url_serves_now() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new(cache.path());
        let doc = document(server.url("/rfc9449.html"), sha256(b"an old pin"));

        assert_eq!(fetcher.fetch_hash(&doc).unwrap(), sha256(SPEC));
        // Pinning the printed hash then needs no second download.
        let repinned = document(doc.url.clone(), sha256(SPEC));
        assert!(fetcher.fetch_pinned(&repinned).unwrap().from_cache);
        assert_eq!(server.requests(), 1);
    }
}
