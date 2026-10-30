//! License detection for `codecite add`: the SPDX id of each license file
//! at a checkout's top level, matched against the texts in `licenses/`, and
//! the license class the design's table gives it.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::sync::OnceLock;

use spdx::detection::{Store, TextData};

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
}
