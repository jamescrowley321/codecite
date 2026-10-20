# Identity Corpus Manifest and Lookup Check Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #28, the M2 acceptance target. Add `corpora/identity.toml`, the identity corpus from `docs/superpowers/specs/2026-10-17-identity-corpus-design.md`: identity-model's primary checkout as a `local` source, every OpenID spec and RFC the design lists, pinned by URL and sha256 with `use = "quote"` and aliases, and the three older versions identity-model has cited, marked `current = false`. Add a lookup check that indexes the corpus and asserts that the issue's seven citations resolve to the right text.

**Architecture:** No library code changes. #21 to #27 already index `local` and `document` sources, keep several pinned versions of one document, store aliases and read written citations, so this issue is data plus two tests:
- `corpora/identity.toml`: 1 local source and 39 document sources (11 OpenID, including the 3 older versions, and 28 RFCs). The hashes come from `codecite index identity --print-hashes`.
- `crates/codecite/tests/identity.rs`:
  - `the_manifest_lists_identity_model_and_the_specs` runs in CI. It parses the real manifest with `codecite_index::Manifest::load` and checks its shape: the local source, `quote` on every document, the three older versions, and every RFC the design lists. It downloads nothing.
  - `the_identity_corpus_indexes_and_its_lookups_resolve` is `#[ignore]`d, because it downloads every pinned spec and indexes `~/repos/auth/identity-model`. It runs the real binary and prints what it reads, so its `--nocapture` output goes in the PR body.

**Tech Stack:** Rust 1.99.1 (edition 2024). No new dependencies: `codecite-index` is already a normal dependency of the `codecite` package, so its integration tests can use it, and `serde_json` and `tempfile` are already there. `cargo deny check` is unchanged. Prototyped: every gate passes (275 passed, 7 ignored; deny ok), and the lookup check passes against the real corpus (output in Task 2, Step 3).

**Spec:** Issue #28; `docs/superpowers/specs/2026-10-17-identity-corpus-design.md` ("The corpus", "Versions") and `docs/design.md#corpus-manifests-and-licensing`. The gaps the issue and spec leave are settled under Global Constraints. Task 3 writes the lasting ones into design.md.

## Global Constraints

- **Older versions.** All three exist on openid.net, so nothing is substituted and the issue need not block:
  - `openid-connect-core-1_0-final.html`, Core 1.0 dated February 25, 2014, with no errata;
  - `openid-connect-backchannel-1_0-final.html`, Back-Channel Logout 1.0 dated September 12, 2022, before errata set 1 (December 15, 2023);
  - `openid-connect-basic-1_0-32.html`, Basic draft 32.

  `check_versions` requires exactly one `current = true` version per document name. Basic is not in the design's current list, so the latest Basic draft (`openid-connect-basic-1_0.html`, draft 47) is pinned as current, and draft 32 is pinned as the older version beside it.
- **Names.** A document's name is its URL's file stem: `rfc9449`, `openid-connect-core-1_0`, `fapi-security-profile-2_0`. These are the names the design's citations use. Both FAPI pages are fetched from their unsuffixed URLs, which serve the same bytes as their `-final` URLs.
- **Aliases.** Each alias names exactly one document (`index_corpus` refuses a duplicate). The aliases avoid names two documents could share: `OIDC Registration` rather than `Dynamic Client Registration`, which RFC 7591 also is. Only current versions carry aliases. They are the same document name, so a written citation reads the current version.
- **Relations.** Taken by hand from rfc-editor.org's `rfcNNNN.json`: no RFC in the list is obsoleted. `updated_by` holds one name, so it names the latest RFC that updates the document: 2119 → rfc8174, 6749 → rfc9700, 6750 → rfc9700, 7518 → rfc9864, 7519 → rfc8725. RFC 9864 is not in the corpus. A relation names a document; it need not be indexed.
- **Licenses.** `IETF Trust` for RFCs and `OpenID Foundation` for OpenID specs, `use = "quote"` for every document; `Apache-2.0` and `use = "adapt"` for identity-model.
- **Real corpora vs fixtures.** The ignored check sets only `XDG_DATA_HOME` (a temp dir), so the corpus file never touches `~/.local/share/codecite`. `HOME` stays real, so `~/repos/auth/identity-model` expands, and fetched documents stay in the user's cache (`~/.cache/codecite/documents`). They are checked against their pins there, so a second run downloads nothing. Every other test keeps using fixtures.
- The workspace forbids `unsafe_code`. Test files need no doc comments, but these keep the house style.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`. Before the PR, also run `cargo test -p codecite --test identity -- --ignored --nocapture` and put its output in the PR body.

## Review Focus

- Pins: every `sha256` is what `--print-hashes` printed on 2026-10-20 (`codecite index identity --print-hashes` marks each `(pinned)`).
- The older versions really are the older text: the dates above come from each page's own header.
- The bare `§2.6` check in the lookup test: three current documents have a §2.6, so it must fail with all three as candidates and never guess.

---

### Task 1: The identity manifest (`corpora/identity.toml`)

**Files:**
- Create: `crates/codecite/tests/identity.rs`, `corpora/identity.toml`

- [ ] **Step 1: Write the failing test**

Create `crates/codecite/tests/identity.rs`:

`````rust
//! The identity corpus (`corpora/identity.toml`): identity-model and the
//! OAuth and OpenID specs it cites. These tests read the real manifest, not
//! a fixture. The lookup check downloads every pinned document and indexes
//! the identity-model checkout, so it is ignored by default:
//!
//! ```text
//! cargo test -p codecite --test identity -- --ignored --nocapture
//! ```
//!
//! Linux only, since other platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};

use codecite_index::{LicenseClass, Manifest, Source};

/// The workspace root, where `corpora/` lives.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifest() -> Manifest {
    Manifest::load(&workspace().join("corpora/identity.toml")).unwrap()
}

#[test]
fn the_manifest_lists_identity_model_and_the_specs() {
    let manifest = manifest();
    assert_eq!(manifest.name, "identity");

    let Source::Local(local) = &manifest.sources[0] else {
        panic!("the first source is identity-model's checkout");
    };
    assert!(local.path.ends_with("repos/auth/identity-model"));
    assert_eq!(local.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(local.license_class, LicenseClass::Adapt);

    let documents: Vec<_> = manifest
        .sources
        .iter()
        .filter_map(|source| match source {
            Source::Document(doc) => Some(doc),
            _ => None,
        })
        .collect();
    assert_eq!(documents.len() + 1, manifest.sources.len());
    for doc in &documents {
        assert_eq!(doc.license_class, LicenseClass::Quote, "{}", doc.name);
        assert!(doc.license.is_some(), "{}", doc.name);
    }

    // The older versions identity-model has cited, kept beside the current ones.
    let older: Vec<&str> = documents
        .iter()
        .filter(|doc| !doc.current)
        .map(|doc| doc.url.as_str())
        .collect();
    assert_eq!(
        older,
        [
            "https://openid.net/specs/openid-connect-core-1_0-final.html",
            "https://openid.net/specs/openid-connect-backchannel-1_0-final.html",
            "https://openid.net/specs/openid-connect-basic-1_0-32.html",
        ]
    );

    // Every RFC the design lists, current.
    for number in [
        2119, 6749, 6750, 7009, 7515, 7517, 7518, 7519, 7521, 7523, 7591, 7592, 7636, 7638, 7662,
        8174, 8414, 8628, 8693, 8705, 8707, 8725, 9101, 9126, 9207, 9396, 9449, 9700,
    ] {
        let name = format!("rfc{number}");
        assert!(
            documents.iter().any(|doc| doc.name == name && doc.current),
            "{name} is missing"
        );
    }
}
`````

> **Learn: reading specs as data.** The manifest is plain data, so the test reads it through the same `Manifest::load` the CLI uses, with its validation included, instead of matching on TOML text. `let Source::Local(local) = &manifest.sources[0] else { panic!(…) };` is a `let`-`else`: it binds when the pattern matches and runs the `else` block, which must diverge, when it doesn't. `filter_map` with a `match` picks one enum variant out of a list and drops the rest.

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p codecite --test identity`
Expected: FAIL. `the_manifest_lists_identity_model_and_the_specs` panics at `Manifest::load(...).unwrap()` with `No such file or directory`.

- [ ] **Step 3: Write the manifest**

Create `corpora/identity.toml`. The hashes are the ones `codecite index identity --print-hashes` printed. To re-pin, write the file with any 64-hex `sha256` per URL (no two the same), run `--print-hashes` and copy each hash in.

`````toml
# The identity corpus: identity-model and the OAuth and OpenID specs it
# cites. See docs/superpowers/specs/2026-10-17-identity-corpus-design.md.
#
# Index it with `codecite index identity`. Documents are pinned by the
# sha256 of their bytes; `codecite index identity --print-hashes` prints
# the hash each URL serves now. Relations (`obsoleted_by`, `updated_by`)
# are written by hand from rfc-editor.org; `updated_by` names the latest
# RFC that updates a document.

name = "identity"

[[source]]
kind = "local"
path = "~/repos/auth/identity-model"
license = "Apache-2.0"
use = "adapt"

# OpenID Foundation specifications, from openid.net.

[[source]]
kind = "document"
name = "openid-connect-core-1_0"
url = "https://openid.net/specs/openid-connect-core-1_0.html"
sha256 = "4d016752a645e8a3c259baa4173b5a90cb6c82898ce868fc8444adde262a5aed"
license = "OpenID Foundation"
use = "quote"
current = true
aliases = ["OIDC Core", "Core", "OpenID Connect Core", "OpenID Connect Core 1.0"]

# Core 1.0 as first published (February 2014), before errata.
[[source]]
kind = "document"
name = "openid-connect-core-1_0"
url = "https://openid.net/specs/openid-connect-core-1_0-final.html"
sha256 = "c9a06af238e2d3a43f5a9243bf45b1c68d8068df0341a10b480c68987d482a55"
license = "OpenID Foundation"
use = "quote"
current = false

[[source]]
kind = "document"
name = "openid-connect-discovery-1_0"
url = "https://openid.net/specs/openid-connect-discovery-1_0.html"
sha256 = "0401a07b35de50e914e195aa2c3c64cc4f6542464381c885d24a2623fbf75ec2"
license = "OpenID Foundation"
use = "quote"
current = true
aliases = ["OIDC Discovery", "Discovery", "OpenID Connect Discovery 1.0"]

[[source]]
kind = "document"
name = "openid-connect-registration-1_0"
url = "https://openid.net/specs/openid-connect-registration-1_0.html"
sha256 = "c151a4abab7704f2efa9b00530dcfeeefb3de6acbdbdf5ba05c9903f1de2a318"
license = "OpenID Foundation"
use = "quote"
current = true
aliases = ["OIDC Registration", "OIDC Dynamic Client Registration", "OpenID Connect Dynamic Client Registration 1.0"]

[[source]]
kind = "document"
name = "openid-connect-rpinitiated-1_0"
url = "https://openid.net/specs/openid-connect-rpinitiated-1_0.html"
sha256 = "246432dafaa5556d896fa9b9ae4284b213ff6921b33478ed3b46bd6bd4aab804"
license = "OpenID Foundation"
use = "quote"
current = true
aliases = ["RP-Initiated Logout", "OIDC RP-Initiated Logout", "OpenID Connect RP-Initiated Logout 1.0"]

[[source]]
kind = "document"
name = "openid-connect-backchannel-1_0"
url = "https://openid.net/specs/openid-connect-backchannel-1_0.html"
sha256 = "3b5c933d5269047b39a65166d5c4d8ff6852b06451f54ed2531b8f97299fdc50"
license = "OpenID Foundation"
use = "quote"
current = true
aliases = ["Back-Channel Logout", "OIDC Back-Channel Logout", "Back-Channel Logout 1.0", "OpenID Connect Back-Channel Logout 1.0"]

# Back-Channel Logout 1.0 before errata set 1 (September 2022).
[[source]]
kind = "document"
name = "openid-connect-backchannel-1_0"
url = "https://openid.net/specs/openid-connect-backchannel-1_0-final.html"
sha256 = "02343424f9e52edf8c78173540e25da913ad2784ea292dbf7980222ab1b18a22"
license = "OpenID Foundation"
use = "quote"
current = false

# The latest Basic Client Profile draft, current so that draft 32 can be pinned as older.
[[source]]
kind = "document"
name = "openid-connect-basic-1_0"
url = "https://openid.net/specs/openid-connect-basic-1_0.html"
sha256 = "a75b730ccbf22213e5e6c9fef7341fa8147bfbfa5141cc8c045d849d46d1910d"
license = "OpenID Foundation"
use = "quote"
current = true
aliases = ["OIDC Basic", "Basic Client Profile", "OpenID Connect Basic Client Implementer's Guide 1.0"]

# Basic Client Profile draft 32 (December 2013).
[[source]]
kind = "document"
name = "openid-connect-basic-1_0"
url = "https://openid.net/specs/openid-connect-basic-1_0-32.html"
sha256 = "ea3b745387c6589d5d57f59e5b145ee292960b7c38f736e706be17bfcd4f8c03"
license = "OpenID Foundation"
use = "quote"
current = false

[[source]]
kind = "document"
name = "fapi-security-profile-2_0"
url = "https://openid.net/specs/fapi-security-profile-2_0.html"
sha256 = "26a49ad19b1f2b19ecc1cd9b825b4d5012f5e03a5dc8cb0dbd26462d39da465c"
license = "OpenID Foundation"
use = "quote"
current = true
aliases = ["FAPI 2.0 Security Profile", "FAPI Security Profile 2.0"]

[[source]]
kind = "document"
name = "fapi-message-signing-2_0"
url = "https://openid.net/specs/fapi-message-signing-2_0.html"
sha256 = "dc59c7d55017078967d4f230b2a1858686325c8cc5339bf5a6e79b806bee59b3"
license = "OpenID Foundation"
use = "quote"
current = true
aliases = ["FAPI 2.0 Message Signing", "FAPI Message Signing 2.0"]

# IETF RFCs, from rfc-editor.org. RFC 2119 and RFC 8174 define the requirement keywords.

[[source]]
kind = "document"
name = "rfc2119"
url = "https://www.rfc-editor.org/rfc/rfc2119.html"
sha256 = "3eb4d7bb487a09ec3936e386d1b61a620a9f5716f701f27676ff1564d18d2a5a"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["BCP 14"]
updated_by = "rfc8174"

[[source]]
kind = "document"
name = "rfc6749"
url = "https://www.rfc-editor.org/rfc/rfc6749.html"
sha256 = "535362fa3b4ca668d4734c244c6ed8c811f2ccd7ea1612a4201e5d8922a74b4e"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["OAuth 2.0"]
updated_by = "rfc9700"

[[source]]
kind = "document"
name = "rfc6750"
url = "https://www.rfc-editor.org/rfc/rfc6750.html"
sha256 = "c458bb43ff32efb811466120efdec411010cdde537e59948342240a32dcd5a1c"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["Bearer Token Usage"]
updated_by = "rfc9700"

[[source]]
kind = "document"
name = "rfc7009"
url = "https://www.rfc-editor.org/rfc/rfc7009.html"
sha256 = "af01723527d58beb02d56c236d4cec1e9b702da239e9a91920e9ecc839c01d8f"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["Token Revocation"]

[[source]]
kind = "document"
name = "rfc7515"
url = "https://www.rfc-editor.org/rfc/rfc7515.html"
sha256 = "88fcf41b04710963f704f3dedb64425abacdd34308660f1d35688a2ac3ba0bd5"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["JWS"]

[[source]]
kind = "document"
name = "rfc7517"
url = "https://www.rfc-editor.org/rfc/rfc7517.html"
sha256 = "d0d5d43a2b55352c60da26cfbcffd12632ee1e45182347a67af9858fb37c3c01"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["JWK"]

[[source]]
kind = "document"
name = "rfc7518"
url = "https://www.rfc-editor.org/rfc/rfc7518.html"
sha256 = "2fe340b744196a10cc9472ab8987c8cd972d10f2c597b7e292c6dedd00c2bf13"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["JWA"]
updated_by = "rfc9864"

[[source]]
kind = "document"
name = "rfc7519"
url = "https://www.rfc-editor.org/rfc/rfc7519.html"
sha256 = "d0ca0d6c6f8b7dfcad6e13c55086694d7ee69665bb828dcac6362d1587b4b53b"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["JWT"]
updated_by = "rfc8725"

[[source]]
kind = "document"
name = "rfc7521"
url = "https://www.rfc-editor.org/rfc/rfc7521.html"
sha256 = "cf1310a91c058bdea561904df387ae54b732c2e254d2c0d9c1a6eedefda63843"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["Assertion Framework"]

[[source]]
kind = "document"
name = "rfc7523"
url = "https://www.rfc-editor.org/rfc/rfc7523.html"
sha256 = "718fd1261b19af816ac064386f5373d63ed2b7891a0bb057ebbe29f628202f24"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["JWT Client Authentication"]

[[source]]
kind = "document"
name = "rfc7591"
url = "https://www.rfc-editor.org/rfc/rfc7591.html"
sha256 = "a946f977f181ea69b2c7566584fa7b6fac90a158828ef4cf8168f87be6b2c90a"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["OAuth Dynamic Client Registration"]

[[source]]
kind = "document"
name = "rfc7592"
url = "https://www.rfc-editor.org/rfc/rfc7592.html"
sha256 = "ddc32ae10ca972ef33f42e1b1ec7216579da53be84d5df19abf83465d33dffeb"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["OAuth Dynamic Client Registration Management"]

[[source]]
kind = "document"
name = "rfc7636"
url = "https://www.rfc-editor.org/rfc/rfc7636.html"
sha256 = "1dd39bd17081e0ed18c2cf304b87c79061c51e505d2ff5e12ffbd0d182fe5b20"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["PKCE"]

[[source]]
kind = "document"
name = "rfc7638"
url = "https://www.rfc-editor.org/rfc/rfc7638.html"
sha256 = "13e604d659d8c8fdb8ddbe6097f6073c93f3f6fe2d867501d6190480a560125c"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["JWK Thumbprint"]

[[source]]
kind = "document"
name = "rfc7662"
url = "https://www.rfc-editor.org/rfc/rfc7662.html"
sha256 = "8c36ce5246215c16fac436fafda277ee0638566b7610aa5cc6673388009445d7"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["Token Introspection"]

[[source]]
kind = "document"
name = "rfc8174"
url = "https://www.rfc-editor.org/rfc/rfc8174.html"
sha256 = "fa760f13d456fc6d0b95d2d565d4bfa698d3c542e0890c19a2a8c6125e1c3260"
license = "IETF Trust"
use = "quote"
current = true

[[source]]
kind = "document"
name = "rfc8414"
url = "https://www.rfc-editor.org/rfc/rfc8414.html"
sha256 = "05083c732395b4499316f5d030f2f86cc203f9ac8a455af7efb3f27a29b57697"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["Authorization Server Metadata"]

[[source]]
kind = "document"
name = "rfc8628"
url = "https://www.rfc-editor.org/rfc/rfc8628.html"
sha256 = "4230ffd7f528dd641fd2579eee8acbb98da877867e9231998712f19dda1b349f"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["Device Authorization Grant"]

[[source]]
kind = "document"
name = "rfc8693"
url = "https://www.rfc-editor.org/rfc/rfc8693.html"
sha256 = "cad0eccfa08d8c57eb096c2750e6cc783f8697b8771558943971fa58caff0fca"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["Token Exchange"]

[[source]]
kind = "document"
name = "rfc8705"
url = "https://www.rfc-editor.org/rfc/rfc8705.html"
sha256 = "f9ca7c82d30fd90f1ab21bc16d561ae5e3da625e824cc73b97000ae829f2bdb9"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["mTLS"]

[[source]]
kind = "document"
name = "rfc8707"
url = "https://www.rfc-editor.org/rfc/rfc8707.html"
sha256 = "d085d9b8e5665995a3c9702ffb1458ac93c51855624f1109e2c81e467f3e82dd"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["Resource Indicators"]

[[source]]
kind = "document"
name = "rfc8725"
url = "https://www.rfc-editor.org/rfc/rfc8725.html"
sha256 = "7f93df8a106c14e2c9394bd3fc12a488266477c89a1fe2f9182cf0bdb02d7ce0"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["JWT BCP"]

[[source]]
kind = "document"
name = "rfc9101"
url = "https://www.rfc-editor.org/rfc/rfc9101.html"
sha256 = "14a98eb7f65063f0eaa37b0d48ed0e956cb5fd8e38e799a28a119c56554ccb94"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["JAR"]

[[source]]
kind = "document"
name = "rfc9126"
url = "https://www.rfc-editor.org/rfc/rfc9126.html"
sha256 = "63ff5c59d0017c94165d3cc49786fd272edfe2666a4e96c2b65a623a0010b31c"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["PAR"]

[[source]]
kind = "document"
name = "rfc9207"
url = "https://www.rfc-editor.org/rfc/rfc9207.html"
sha256 = "482fca2e2591ac8b30566f952112bfb19887f186c5140251ccd4948a64a3be85"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["Issuer Identification"]

[[source]]
kind = "document"
name = "rfc9396"
url = "https://www.rfc-editor.org/rfc/rfc9396.html"
sha256 = "a71380d14733dd4d5056ecec42598b5b2661b197a4079468ce09c71d1adf2e33"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["RAR"]

[[source]]
kind = "document"
name = "rfc9449"
url = "https://www.rfc-editor.org/rfc/rfc9449.html"
sha256 = "2d8657f072913d55627dc25c02d3ad305bd62bed7f06c78c7d0621768b234784"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["DPoP"]

[[source]]
kind = "document"
name = "rfc9700"
url = "https://www.rfc-editor.org/rfc/rfc9700.html"
sha256 = "c00e2234fcd79581bc29669c60d693afaa42f67d9205bddee848f757ae975419"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["OAuth 2.0 Security BCP"]
`````

- [ ] **Step 4: Run it and watch it pass**

Run: `cargo test -p codecite --test identity` → `1 passed`.
Run: `cargo run -q -p codecite -- index identity --print-hashes`. Expected: 39 lines, each ending in `(pinned)`.

- [ ] **Step 5: Commit**

```bash
git add corpora/identity.toml crates/codecite/tests/identity.rs
git commit -m "feat: add the identity corpus manifest (#28)"
```

### Task 2: The lookup check

**Files:**
- Modify: `crates/codecite/tests/identity.rs`

- [ ] **Step 1: Add the ignored lookup check**

`````diff
--- a/crates/codecite/tests/identity.rs
+++ b/crates/codecite/tests/identity.rs
@@ -11,6 +11,7 @@
 #![cfg(target_os = "linux")]
 
 use std::path::{Path, PathBuf};
+use std::process::{Command, Output};
 
 use codecite_index::{LicenseClass, Manifest, Source};
 
@@ -76,3 +77,128 @@
         );
     }
 }
+
+/// A citation, the citation `read` should resolve it to (its pinned hash
+/// cut out), the section's title and words its text holds.
+struct Lookup {
+    citation: &'static str,
+    resolves_to: (&'static str, &'static str),
+    title: &'static str,
+    holds: &'static str,
+}
+
+const LOOKUPS: &[Lookup] = &[
+    Lookup {
+        citation: "OIDC Core §3.1.3.7 step 13",
+        resolves_to: (
+            "openid-connect-core-1_0@sha256:",
+            "#section-3.1.3.7/step-13",
+        ),
+        title: "ID Token Validation",
+        holds: "the Client SHOULD check the auth_time Claim value",
+    },
+    Lookup {
+        citation: "RFC 9449 §4.3",
+        resolves_to: ("rfc9449@sha256:", "#section-4.3"),
+        title: "Checking DPoP Proofs",
+        holds: "To validate a DPoP proof, the receiving server MUST ensure",
+    },
+    Lookup {
+        citation: "RFC 6749 §4.1.2.1",
+        resolves_to: ("rfc6749@sha256:", "#section-4.1.2.1"),
+        title: "Error Response",
+        holds: "mismatching\n   redirection URI",
+    },
+    Lookup {
+        citation: "RFC 7636 §4.6",
+        resolves_to: ("rfc7636@sha256:", "#section-4.6"),
+        title: "Server Verifies code_verifier before Returning the Tokens",
+        holds: "calculating the code challenge from the received",
+    },
+    Lookup {
+        citation: "Back-Channel Logout §2.6",
+        resolves_to: ("openid-connect-backchannel-1_0@sha256:", "#section-2.6"),
+        title: "Logout Token Validation",
+        holds: "the RP MUST validate the Logout Token as follows",
+    },
+    Lookup {
+        citation: "RFC 8414 §3",
+        resolves_to: ("rfc8414@sha256:", "#section-3"),
+        title: "Obtaining Authorization Server Metadata",
+        holds: "Authorization servers supporting metadata MUST make a JSON document",
+    },
+    // A bare § resolves when exactly one current document has the step.
+    Lookup {
+        citation: "§3.1.3.7 step 12",
+        resolves_to: (
+            "openid-connect-core-1_0@sha256:",
+            "#section-3.1.3.7/step-12",
+        ),
+        title: "ID Token Validation",
+        holds: "If the acr Claim was requested",
+    },
+];
+
+/// Runs the binary from the workspace root with its corpus file in `data`.
+/// Fetched documents stay in the user's cache, where they are checked
+/// against their pins, so a second run downloads nothing.
+fn run(data: &Path, args: &[&str]) -> Output {
+    Command::new(env!("CARGO_BIN_EXE_codecite"))
+        .current_dir(workspace())
+        .env("XDG_DATA_HOME", data)
+        .args(args)
+        .output()
+        .unwrap()
+}
+
+#[test]
+#[ignore = "downloads every pinned spec and indexes ~/repos/auth/identity-model; run with `cargo test -p codecite --test identity -- --ignored --nocapture`"]
+fn the_identity_corpus_indexes_and_its_lookups_resolve() {
+    let data = tempfile::tempdir().unwrap();
+    let indexed = run(data.path(), &["index", "identity"]);
+    let stdout = String::from_utf8_lossy(&indexed.stdout);
+    assert!(
+        indexed.status.success(),
+        "{}",
+        String::from_utf8_lossy(&indexed.stderr)
+    );
+    println!("$ codecite index identity");
+    println!("{}", stdout.lines().skip(1).collect::<Vec<_>>().join("\n"));
+
+    for lookup in LOOKUPS {
+        let read = run(
+            data.path(),
+            &["read", "identity", lookup.citation, "--json"],
+        );
+        assert!(
+            read.status.success(),
+            "{}: {}",
+            lookup.citation,
+            String::from_utf8_lossy(&read.stderr)
+        );
+        let result: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap();
+        let citation = result["citation"].as_str().unwrap();
+        let text = result["text"].as_str().unwrap();
+        println!("\n$ codecite read identity {:?}", lookup.citation);
+        println!("{citation}  {}  {}", result["title"], result["strength"]);
+        println!("{}", text.lines().next().unwrap_or_default());
+
+        let (name, anchor) = lookup.resolves_to;
+        assert!(
+            citation.starts_with(name) && citation.ends_with(anchor),
+            "{}: {citation}",
+            lookup.citation
+        );
+        assert_eq!(result["title"], lookup.title, "{}", lookup.citation);
+        assert!(text.contains(lookup.holds), "{}: {text}", lookup.citation);
+    }
+
+    // Three current documents have a §2.6, so a bare one is not guessed.
+    let ambiguous = run(data.path(), &["read", "identity", "§2.6"]);
+    let stderr = String::from_utf8_lossy(&ambiguous.stderr);
+    println!("\n$ codecite read identity \"§2.6\"\n{}", stderr.trim_end());
+    assert!(!ambiguous.status.success());
+    for name in ["openid-connect-backchannel-1_0@", "rfc8725@", "rfc9700@"] {
+        assert!(stderr.contains(name), "{stderr}");
+    }
+}
`````

> **Learn: keeping fixtures and real corpora apart.** A test that needs the network and a checkout on the owner's machine gets `#[ignore = "why, and how to run it"]`. `cargo test` skips it, `-- --ignored` runs it, and the reason prints in the test list. The check drives the real binary (`env!("CARGO_BIN_EXE_codecite")`, which Cargo sets for integration tests) and gives it a scratch `XDG_DATA_HOME`, so it never overwrites the owner's own `identity.db`. The lookup table is a `const` slice of a small struct with `&'static str` fields. Each case is one row, and a failing assert names the citation it was checking.

- [ ] **Step 2: Run the gates**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`
Expected: all pass. The identity test binary reports `1 passed; 0 failed; 1 ignored`.

- [ ] **Step 3: Run the lookup check against the real corpus**

Run: `cargo test -q -p codecite --test identity -- --include-ignored --nocapture`
Expected (the prototype's run; elapsed time varies):

`````text

running 2 tests
.$ codecite index identity
documents: 865 written, 0 unchanged, 0 removed
chunks: 12187 written
fetched: 0 downloaded, 39 from the cache
skipped: 39 ignored, 7 deny-listed, 4 symlink, 1 binary
elapsed: 6.24s

$ codecite read identity "OIDC Core §3.1.3.7 step 13"
openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-13  "ID Token Validation"  ["SHOULD"]
13. If the auth_time Claim was requested, either through a specific request for this Claim or by using the max_age parameter, the Client SHOULD check the auth_time Claim value and request re-authentication if it determines too much time has elapsed since the last End-User authentication.

$ codecite read identity "RFC 9449 §4.3"
rfc9449@sha256:2d8657f07291#section-4.3  "Checking DPoP Proofs"  ["MUST","SHOULD"]
4.3. Checking DPoP Proofs

$ codecite read identity "RFC 6749 §4.1.2.1"
rfc6749@sha256:535362fa3b4c#section-4.1.2.1  "Error Response"  ["MUST","MUST NOT","REQUIRED","SHOULD","OPTIONAL"]
4.1.2.1.  Error Response

$ codecite read identity "RFC 7636 §4.6"
rfc7636@sha256:1dd39bd17081#section-4.6  "Server Verifies code_verifier before Returning the Tokens"  ["MUST"]
4.6.  Server Verifies code_verifier before Returning the Tokens

$ codecite read identity "Back-Channel Logout §2.6"
openid-connect-backchannel-1_0@sha256:3b5c933d5269#section-2.6  "Logout Token Validation"  ["MUST","MUST NOT","SHOULD"]
2.6. Logout Token Validation

$ codecite read identity "RFC 8414 §3"
rfc8414@sha256:05083c732395#section-3  "Obtaining Authorization Server Metadata"  ["MUST","MAY"]
3.  Obtaining Authorization Server Metadata

$ codecite read identity "§3.1.3.7 step 12"
openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-12  "ID Token Validation"  ["SHOULD"]
12. If the acr Claim was requested, the Client SHOULD check that the asserted Claim Value is appropriate. The meaning and processing of acr Claim Values is out of scope for this specification.

$ codecite read identity "§2.6"
Error: §2.6 names no document and is not in exactly one current document; name one: openid-connect-backchannel-1_0@sha256:3b5c933d5269#section-2.6, rfc8725@sha256:7f93df8a106c#section-2.6, rfc9700@sha256:c00e2234fcd7#section-2.6
.
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.34s
`````

- [ ] **Step 4: Commit**

```bash
git add crates/codecite/tests/identity.rs
git commit -m "test: check the identity corpus lookups (#28)"
```

### Task 3: Docs

**Files:**
- Modify: `docs/design.md`, `README.md`

- [ ] **Step 1: Describe the identity corpus and its check**

`````diff
diff --git a/README.md b/README.md
index ac39443..91e787b 100644
--- a/README.md
+++ b/README.md
@@ -22,7 +22,8 @@ and refuses an index older than the manifest's `max_age_days` unless
 `--allow-stale`. `codecite read <corpus> <citation>` prints the exact text a
 citation points at under its pinned citation, for the citations search prints
 and the spec forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`,
-a spec URL). The MCP server comes next.
+a spec URL). `corpora/identity.toml` is the first real corpus: identity-model and
+the OAuth and OpenID specs it cites. The MCP server comes next.
 
 ## Planned
 
diff --git a/docs/design.md b/docs/design.md
index b4b5075..27cf187 100644
--- a/docs/design.md
+++ b/docs/design.md
@@ -247,7 +247,7 @@ use = "quote"
 current = true                   # several versions of one name may be pinned; one is current
 ```
 
-`kind = "local"` reads a working tree in place (`path = "~/repos/auth/identity-model"`); its results cite the checkout's HEAD and carry `dirty: true` for files with uncommitted changes (modified, staged or untracked). `path` must be the top of a git work tree with at least one commit; anything else is an error. Document relations (`obsoleted_by`, `updated_by`) are written by hand in the manifest and never looked up elsewhere; they win over what an RFC's own header says, since a document published before its successor can't name it. The first real corpus, `corpora/identity.toml`, is identity-model plus the OAuth and OpenID specs it cites; see `docs/superpowers/specs/2026-10-17-identity-corpus-design.md`.
+`kind = "local"` reads a working tree in place (`path = "~/repos/auth/identity-model"`); its results cite the checkout's HEAD and carry `dirty: true` for files with uncommitted changes (modified, staged or untracked). `path` must be the top of a git work tree with at least one commit; anything else is an error. Document relations (`obsoleted_by`, `updated_by`) are written by hand in the manifest and never looked up elsewhere; they win over what an RFC's own header says, since a document published before its successor can't name it. `updated_by` holds one name, so for an RFC updated several times it names the latest. The first real corpus, `corpora/identity.toml`, is identity-model plus the OAuth and OpenID specs it cites, with the older versions it has cited pinned beside the current ones (Core 1.0 as published in 2014, Back-Channel Logout 1.0 before errata set 1, Basic Client Profile draft 32, which keeps the latest Basic draft as its current version); see `docs/superpowers/specs/2026-10-17-identity-corpus-design.md`. Its lookup check, `cargo test -p codecite --test identity -- --ignored --nocapture`, indexes it and reads back a fixed list of citations; tests otherwise use fixtures, never a real corpus.
 
 Each source needs `use`; `license` is optional until SPDX detection lands. A git source needs `url` and a 40-character lowercase `commit`; a local source needs `path` (`~` expands to the home directory); a document needs `name`, `url` and a 64-character lowercase `sha256`, and exactly one version of each document name has `current = true`. `name` is optional for git and local sources. Unknown keys are errors, and every error names the source and the field.
 
`````

- [ ] **Step 2: Commit**

```bash
git add docs/design.md README.md
git commit -m "docs: describe the identity corpus and its lookup check (#28)"
```
