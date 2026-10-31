# License texts

The texts `codecite add` matches a repository's license files against,
one per SPDX id, from the SPDX License List
(<https://github.com/spdx/license-list-data>, tag `v3.29.0`,
`text/<id>.txt`). They are the licenses' own texts, used only to recognize
them; `src/license.rs` embeds them with `include_str!`.

A license missing here is reported as not detected, and `add` proposes
`use = "approach_only"` for it. To recognize another, add its text here and
a row to `LICENSES` in `src/license.rs`.
