//! License detection for `codecite add`: the SPDX id of each license file
//! at a checkout's top level, matched against the texts in `licenses/`, and
//! the license class the design's table gives it.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::sync::OnceLock;

use spdx::detection::{Store, TextData};
use spdx::{Expression, LicenseItem, ParseMode};

use crate::LicenseClass;

/// What `add` proposes for a source's `license` and `use`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedLicense {
    /// The detected ids joined with ` AND `, or `None` when no file matched.
    pub spdx: Option<String>,
    /// The strictest class among them; `approach_only` when none matched.
    pub class: LicenseClass,
}

/// The texts a license file is matched against: (SPDX id, text, class).
const LICENSES: &[(&str, &str, LicenseClass)] = &[
    (
        "0BSD",
        include_str!("../licenses/0BSD.txt"),
        LicenseClass::Adapt,
    ),
    (
        "Apache-2.0",
        include_str!("../licenses/Apache-2.0.txt"),
        LicenseClass::Adapt,
    ),
    (
        "BSD-2-Clause",
        include_str!("../licenses/BSD-2-Clause.txt"),
        LicenseClass::Adapt,
    ),
    (
        "BSD-3-Clause",
        include_str!("../licenses/BSD-3-Clause.txt"),
        LicenseClass::Adapt,
    ),
    (
        "ISC",
        include_str!("../licenses/ISC.txt"),
        LicenseClass::Adapt,
    ),
    (
        "MIT",
        include_str!("../licenses/MIT.txt"),
        LicenseClass::Adapt,
    ),
    (
        "Unlicense",
        include_str!("../licenses/Unlicense.txt"),
        LicenseClass::Adapt,
    ),
    (
        "Zlib",
        include_str!("../licenses/Zlib.txt"),
        LicenseClass::Adapt,
    ),
    (
        "MPL-2.0",
        include_str!("../licenses/MPL-2.0.txt"),
        LicenseClass::ApproachOnly,
    ),
    (
        "LGPL-2.1-only",
        include_str!("../licenses/LGPL-2.1-only.txt"),
        LicenseClass::ApproachOnly,
    ),
    (
        "LGPL-3.0-only",
        include_str!("../licenses/LGPL-3.0-only.txt"),
        LicenseClass::ApproachOnly,
    ),
    (
        "GPL-2.0-only",
        include_str!("../licenses/GPL-2.0-only.txt"),
        LicenseClass::ApproachOnly,
    ),
    (
        "GPL-3.0-only",
        include_str!("../licenses/GPL-3.0-only.txt"),
        LicenseClass::ApproachOnly,
    ),
    (
        "AGPL-3.0-only",
        include_str!("../licenses/AGPL-3.0-only.txt"),
        LicenseClass::ApproachOnly,
    ),
];

/// Below this score a file is not that license.
const MIN_SCORE: f32 = 0.9;

/// A license file larger than this is read only up to it.
const MAX_LEN: u64 = 1024 * 1024;

/// The store of [`LICENSES`], built on first use.
fn store() -> &'static Store {
    static STORE: OnceLock<Store> = OnceLock::new();
    STORE.get_or_init(|| {
        let mut store = Store::new();
        for (id, text, _) in LICENSES {
            store.add_license((*id).to_owned(), TextData::new(text));
        }
        store
    })
}

/// Whether `name` is a license file's name: one starting with `license`,
/// `licence`, `copying` or `unlicense`, in any case.
fn is_license_file(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["license", "licence", "copying", "unlicense"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Matches the license files at `dir`'s top level. Only regular files
/// count: a link is never followed, so nothing outside `dir` is read.
pub fn detect_license(dir: &Path) -> io::Result<DetectedLicense> {
    let mut ids = BTreeSet::new();
    let mut class = None;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_license_file(&name) || !fs::symlink_metadata(entry.path())?.is_file() {
            continue;
        }
        let mut bytes = Vec::new();
        fs::File::open(entry.path())?
            .take(MAX_LEN)
            .read_to_end(&mut bytes)?;
        let found = store().analyze(&TextData::new(&String::from_utf8_lossy(&bytes)));
        if found.score < MIN_SCORE {
            continue;
        }
        if let Some((id, _, found_class)) = LICENSES.iter().find(|(id, ..)| *id == found.name) {
            ids.insert(*id);
            if class != Some(LicenseClass::ApproachOnly) {
                class = Some(*found_class);
            }
        }
    }
    let spdx = (!ids.is_empty()).then(|| ids.into_iter().collect::<Vec<_>>().join(" AND "));
    Ok(DetectedLicense {
        spdx,
        class: class.unwrap_or(LicenseClass::ApproachOnly),
    })
}

/// SPDX ids outside the GNU and MPL families whose terms keep `adapt` out:
/// weak copyleft the SPDX data doesn't flag as copyleft, and
/// source-available licenses. Only these exact ids are covered; terms with
/// no SPDX id, such as the Commons Clause, are the person's call.
const NOT_ADAPTABLE: &[&str] = &[
    "BUSL-1.1",
    "CDDL-1.0",
    "CDDL-1.1",
    "CPL-1.0",
    "Elastic-2.0",
    "EPL-1.0",
    "EPL-2.0",
    "PolyForm-Noncommercial-1.0.0",
    "PolyForm-Small-Business-1.0.0",
    "SSPL-1.0",
];

/// Whether the SPDX id `id` forbids adapting code: copyleft, as the SPDX
/// data flags it or as [`LICENSES`] classes it, or source-available.
fn forbids_adapt(id: spdx::LicenseId) -> bool {
    id.is_copyleft()
        || NOT_ADAPTABLE.contains(&id.name)
        || LICENSES
            .iter()
            .any(|(name, _, class)| *name == id.name && *class == LicenseClass::ApproachOnly)
}

/// The licenses that keep `use = "adapt"` out of a source licensed under
/// `license`, when it is an SPDX expression that no choice of its `OR`
/// branches meets without one of them: `GPL-3.0-only` or
/// `MIT AND GPL-2.0-only`, but not `MIT OR GPL-3.0-only`. A `license`
/// that isn't an SPDX expression can't show an `OR` that avoids them, so
/// any word in it that reads as such a license, as leniently as a whole
/// expression is read, keeps `adapt` out: `GPL-3.0-only AND IETF Trust`,
/// `gpl-3.0-only AND IETF Trust`. `None` when the expression allows
/// adapting, and for text that names no such license (`IETF Trust`), whose
/// class is the person's call.
pub(crate) fn forbids_adapting(license: &str) -> Option<Vec<String>> {
    let license = &canonical_ids(license);
    let names: BTreeSet<String> = match forbidding(license) {
        Some(names) => names,
        None => license
            .split(|c: char| !is_id_char(c))
            .filter(|word| !word.is_empty())
            .filter_map(forbidding)
            .flatten()
            .collect(),
    };
    (!names.is_empty()).then(|| names.into_iter().collect())
}

/// Whether `c` can be part of a license id: `GPL-2.0+`, `LicenseRef-x`.
fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '+')
}

/// `license` with each word that is an SPDX license id in another case
/// (`gpl-3.0-only`, `Mit`) spelt as the SPDX list spells it, since ids are
/// case-sensitive even in lax mode, and anything else as it is.
fn canonical_ids(license: &str) -> String {
    let mut out = String::with_capacity(license.len());
    let mut rest = license;
    while let Some(c) = rest.chars().next() {
        if !is_id_char(c) {
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let end = rest.find(|c: char| !is_id_char(c)).unwrap_or(rest.len());
        let (word, after) = rest.split_at(end);
        let base = word.trim_end_matches('+');
        match spdx::identifiers::LICENSES
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(base))
        {
            Some(l) => {
                out.push_str(l.name);
                out.push_str(&word[base.len()..]);
            }
            None => out.push_str(word),
        }
        rest = after;
    }
    out
}

/// The licenses in the SPDX expression `license`, read leniently, that no
/// choice of its `OR` branches avoids and that forbid adapting; `None` when
/// it doesn't parse.
fn forbidding(license: &str) -> Option<BTreeSet<String>> {
    let expression = Expression::parse_mode(license, ParseMode::LAX).ok()?;
    let failures = expression
        .evaluate_with_failures(|req| match &req.license {
            LicenseItem::Spdx { id, .. } => !forbids_adapt(*id),
            LicenseItem::Other { .. } => true,
        })
        .err()
        .unwrap_or_default();
    Some(failures.iter().map(|f| f.req.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(id: &str) -> &'static str {
        LICENSES.iter().find(|(i, ..)| *i == id).unwrap().1
    }

    fn mit() -> String {
        text("MIT").replace("<year> <copyright holders>", "2026 Someone")
    }

    fn detect(files: &[(&str, &str)]) -> DetectedLicense {
        let dir = tempfile::tempdir().unwrap();
        for (name, contents) in files {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        detect_license(dir.path()).unwrap()
    }

    fn detected(spdx: Option<&str>, class: LicenseClass) -> DetectedLicense {
        DetectedLicense {
            spdx: spdx.map(str::to_owned),
            class,
        }
    }

    #[test]
    fn license_mit_with_its_copyright_line_filled_in() {
        assert_eq!(
            detect(&[("LICENSE", &mit())]),
            detected(Some("MIT"), LicenseClass::Adapt)
        );
    }

    #[test]
    fn license_gpl_is_approach_only() {
        assert_eq!(
            detect(&[("LICENSE", text("GPL-3.0-only"))]),
            detected(Some("GPL-3.0-only"), LicenseClass::ApproachOnly)
        );
    }

    #[test]
    fn license_files_are_joined_sorted() {
        assert_eq!(
            detect(&[
                ("LICENSE-MIT", &mit()),
                ("LICENSE-APACHE", text("Apache-2.0"))
            ]),
            detected(Some("Apache-2.0 AND MIT"), LicenseClass::Adapt)
        );
    }

    #[test]
    fn license_strictest_class_wins() {
        assert_eq!(
            detect(&[("LICENSE-MIT", &mit()), ("COPYING", text("GPL-3.0-only"))]),
            detected(Some("GPL-3.0-only AND MIT"), LicenseClass::ApproachOnly)
        );
    }

    #[test]
    fn license_every_embedded_text_detects_as_itself() {
        for (id, body, class) in LICENSES {
            assert_eq!(
                detect(&[("LICENSE", body)]),
                detected(Some(id), *class),
                "{id}"
            );
        }
    }

    #[test]
    fn license_none_or_unrelated_prose_is_not_detected() {
        let none = detected(None, LicenseClass::ApproachOnly);
        assert_eq!(detect(&[("README.md", &mit())]), none);
        assert_eq!(
            detect(&[(
                "LICENSE",
                "All rights reserved. Ask before using any of this.\n"
            )]),
            none
        );
    }

    #[test]
    fn license_only_top_level_regular_files_count() {
        let none = detected(None, LicenseClass::ApproachOnly);
        assert_eq!(detect(&[("docs/LICENSE", &mit())]), none);
        assert_eq!(detect(&[("LICENSE/MIT.txt", &mit())]), none);
    }

    #[cfg(unix)]
    #[test]
    fn license_a_link_is_not_followed() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("MIT"), mit()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path().join("MIT"), dir.path().join("LICENSE")).unwrap();
        assert_eq!(
            detect_license(dir.path()).unwrap(),
            detected(None, LicenseClass::ApproachOnly)
        );
    }

    #[test]
    fn copyleft_licenses_forbid_adapting() {
        let forbids = |l: &str| forbids_adapting(l);
        assert_eq!(forbids("GPL-3.0-only"), Some(vec!["GPL-3.0-only".into()]));
        assert_eq!(
            forbids("MIT AND GPL-2.0-only"),
            Some(vec!["GPL-2.0-only".into()])
        );
        // Each license is named once, sorted.
        assert_eq!(
            forbids("GPL-3.0-only AND MIT AND MPL-2.0 AND GPL-3.0-only"),
            Some(vec!["GPL-3.0-only".into(), "MPL-2.0".into()])
        );
        for l in [
            "AGPL-3.0-or-later",
            "LGPL-2.1-only",
            "MPL-2.0",
            "EPL-2.0",
            "SSPL-1.0",
            "GPL-2.0-only WITH Classpath-exception-2.0",
            // How people write them, read leniently.
            "GPL-3.0",
            "GPL-2.0+",
            "PolyForm-Noncommercial-1.0.0",
        ] {
            assert!(forbids(l).is_some(), "{l}");
        }
        // A copyleft id beside text that isn't SPDX still forbids it: the
        // expression can't be judged, so no `OR` can be shown to avoid it.
        assert_eq!(
            forbids("GPL-3.0-only AND IETF Trust"),
            Some(vec!["GPL-3.0-only".into()])
        );
        assert_eq!(
            forbids("MIT, AGPL-3.0-or-later (see NOTICE)"),
            Some(vec!["AGPL-3.0-or-later".into()])
        );
        assert_eq!(
            forbids("Custom Terms AND GPL-2.0+"),
            Some(vec!["GPL-2.0-or-later".into()])
        );
        // An id in another case is still that id, alone, in an expression
        // or beside other text.
        for l in [
            "gpl-3.0-only",
            "mit and agpl-3.0-only",
            "gpl-3.0-only AND IETF Trust",
        ] {
            assert!(forbids(l).is_some(), "{l}");
        }
        assert_eq!(
            forbids("gpl-3.0-only AND IETF Trust"),
            Some(vec!["GPL-3.0-only".into()])
        );
    }

    #[test]
    fn permissive_choices_and_other_names_allow_adapting() {
        for l in [
            "MIT",
            "Apache-2.0",
            "MIT OR Apache-2.0",
            "MIT OR GPL-3.0-only",
            "Apache-2.0 AND MIT",
            "CC0-1.0",
            "LicenseRef-Mine",
            "MIT/Apache-2.0",
            "IETF Trust",
            "OpenID Foundation",
            "MIT, see NOTICE",
            "mit OR gpl-3.0-only",
            "Apache-2.0 AND café terms",
            "",
        ] {
            assert_eq!(forbids_adapting(l), None, "{l}");
        }
    }
}
