//! The spec adapter for codecite. A document source indexed with
//! `adapter = "spec"` is split into numbered sections by this crate's RFC
//! and OpenID parsers; the rest of the spec semantics live here too: BCP 14
//! keyword detection, the spec citation forms people write (`RFC 6749
//! §4.1`, aliases, Markdown links to a section), superseded-by,
//! `requirements` and `diff`.
//!
//! Core keeps what specs share with guidance: the schema, the canonical
//! section citation and reading it. This crate depends on `codecite-core`
//! alone, through its public read API.
