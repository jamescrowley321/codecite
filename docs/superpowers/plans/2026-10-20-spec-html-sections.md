# Spec HTML Sections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #23: `codecite-index` splits spec HTML into numbered sections, for the three layouts in the identity corpus (xml2rfc v3 RFCs such as RFC 9449, the older rfc-editor.org RFC HTML such as RFC 6749, and the xml2rfc-generated OpenID pages such as Connect Core 1.0). Each section has its number, title, the document's own anchor, its parent section, and its text with a line range in a plain-text rendering of the page. Nothing is stored yet: #25 indexes the sections.

**Architecture:** One pure function, `split_spec(html: &str) -> Result<SpecDocument, SpecError>`, in a new `spec` module. It does no I/O: the caller fetches the bytes (the `Fetcher` from #22) and passes the HTML in. `spec/render.rs` parses with `scraper`, detects the layout from the markup around section numbers, and walks the tree once without recursion (`ego_tree`'s `traverse`, `Open`/`Close` edges), writing plain-text lines and recording each heading's line. `spec.rs` turns the lines and headings into sections: a section runs from its heading to the line before the next heading, numbered or not, and references (and anything numbered inside them) are dropped.

**Tech Stack:** Rust 1.99.1 (edition 2024). New dependencies of `codecite-index`: `scraper` 0.27 (ISC, `default-features = false`), an HTML parser built on html5ever (MIT OR Apache-2.0), and `ego-tree` 0.11 (ISC), the tree type `scraper` exposes, named directly for `NodeId` and `iter::Edge`. `scraper` pulls Servo's `selectors` and `cssparser`, which are MPL-2.0: `cargo deny` gets an exception for each of the four MPL crates, as it already has for `option-ext`, and a skip for `syn` 2, which their proc macros still use. Prototyped: all gates pass, and so do the three `--ignored` tests against the real documents.

**Spec:** Issue #23; `docs/superpowers/specs/2026-10-17-identity-corpus-design.md` (Spec structure, Left to the builder); `docs/design.md#indexing-and-retrieval` (step 3).

## Global Constraints

- A section is a numbered section: `4.3`, `3.1.3.7`, `A.1` (digits, or one capital letter, then dot-separated digits). Front matter, the table of contents, unnumbered sections and the references (a section titled `References`, `Normative References` or `Informative References`, and every section numbered inside one) are not sections.
- Each section stores `number`, `title`, `anchor` (without `#`: `section-4.3`, `appendix-A.1`, `IDTokenValidation`), `parent` (index of the enclosing section), `start_line`/`end_line` (1-based, inclusive, into `SpecDocument::text`) and `text` (exactly those lines). Nested sections are their own sections, and a section's text stops where its first subsection starts.
- Parsing never fails on malformed HTML; the only error is a page in none of the three layouts.
- Parsing is separate from I/O: `split_spec` takes `&str`. Only the `#[ignore]` tests fetch, through the pinned cache in `std::env::temp_dir()` (`$TMPDIR`, `/var/tmp` in the loop's shell).
- Spec documents are never committed. The fixtures are small hand-written HTML in each layout's structure.
- The workspace forbids `unsafe_code` and warns on `missing_docs`; every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`. Before the PR, also `cargo test -p codecite-index --lib spec:: -- --ignored --nocapture`, with its output in the PR body.

## Review Focus

- Acceptance, against the real pages (`--ignored`): `real_rfc9449` (§4.3 `Checking DPoP Proofs`, anchor `section-4.3`, parent §4, the text reaches check 12), `real_rfc6749` (§4.1.2.1 `Error Response`, parent §4.1.2, no `[Page N]` footers anywhere), `real_openid_connect_core` (§3.1.3.7 `ID Token Validation`, anchor `IDTokenValidation`, parent §3.1.3, the text reaches step 13). Each also checks every section's text is its own line range and that the references are not sections.
- Page furniture in the older RFC layout: each page's header and footer is a `span.grey` and is skipped, so a section spanning a page break reads as one text (`rfc_text_fixture`). rfc-editor.org writes the last page's footer without the span; `page_footers_outside_grey_spans_are_dropped` covers that line, and `[Page 2]` inside indented body text stays.
- Unnumbered headings end the section before them: "Author's Address" after Appendix A.1 in the older layout is an unindented line, not markup, and `rfc_text_fixture` checks A.1 stops before it.
- Malformed and hostile HTML: `malformed_html_still_splits` (unclosed and stray tags), `unknown_pages_are_an_error` (no spec headings; an empty page), `deep_nesting_does_not_overflow_the_stack` (5,000 nested elements on a 256 KiB stack).
- Known limit, not a bug: html5ever's parse is quadratic in nesting depth (100,000 nested `<div>`s take 20 s in a release build, minutes in debug), which is why the deep-nesting test stays at 5,000. Documents are pinned by a hash the owner chose, and real specs nest under 30 deep; design.md says so.

---

### Task 1: Split spec HTML into numbered sections (`codecite-index`)

**Files:**
- Modify: `Cargo.toml`, `crates/codecite-index/Cargo.toml`, `deny.toml`, `Cargo.lock`
- Create: `crates/codecite-index/tests/fixtures/spec/rfc-v3.html`, `rfc-text.html`, `openid.html`
- Create: `crates/codecite-index/src/spec.rs`, `crates/codecite-index/src/spec/render.rs`
- Modify: `crates/codecite-index/src/lib.rs`

**Interfaces:**
- Consumes: `Fetcher::new(cache_dir)`, `Fetcher::fetch_pinned(&DocumentSource) -> Result<Fetched, FetchError>` and `DocumentSource` from #22, in the `#[ignore]` tests only.
- Produces (for #24 steps and #25 indexing): `pub fn split_spec(html: &str) -> Result<SpecDocument, SpecError>`; `SpecDocument { format: SpecFormat, text: String, sections: Vec<SpecSection> }` with `fn section(&self, number: &str) -> Option<&SpecSection>`; `SpecSection { number, title, anchor: String, parent: Option<usize>, start_line: u32, end_line: u32, text: String }`; `SpecFormat::{RfcV3, RfcText, OpenId}`; `SpecError::UnknownFormat`. All re-exported from the crate root.

- [ ] **Step 1: Add the dependencies and the cargo-deny entries**

```diff
diff --git a/Cargo.toml b/Cargo.toml
index 1581df2..d18d717 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -44,3 +44,5 @@ tree-sitter-go = "0.25"
 tree-sitter-c-sharp = "0.23"
 streaming-iterator = "0.1"
 tempfile = "3"
+scraper = { version = "0.27", default-features = false }
+ego-tree = "0.11"
```

```diff
diff --git a/crates/codecite-index/Cargo.toml b/crates/codecite-index/Cargo.toml
index c51780a..ed085ef 100644
--- a/crates/codecite-index/Cargo.toml
+++ b/crates/codecite-index/Cargo.toml
@@ -27,6 +27,8 @@ tree-sitter-javascript.workspace = true
 tree-sitter-go.workspace = true
 tree-sitter-c-sharp.workspace = true
 streaming-iterator.workspace = true
+scraper.workspace = true
+ego-tree.workspace = true
 
 [dev-dependencies]
 rusqlite.workspace = true
```

```diff
diff --git a/deny.toml b/deny.toml
index a4733e0..8725a82 100644
--- a/deny.toml
+++ b/deny.toml
@@ -31,6 +31,13 @@ exceptions = [
     # Mozilla's root certificates as data, via `ureq` -> `webpki-roots`. A
     # permissive data license; it asks only that the license text be kept.
     { crate = "webpki-roots", allow = ["CDLA-Permissive-2.0"] },
+    # Servo's CSS selector engine, via `scraper` (spec HTML). MPL-2.0 is
+    # file-level copyleft: using the crates unmodified places no terms on
+    # codecite's own files.
+    { crate = "cssparser", allow = ["MPL-2.0"] },
+    { crate = "cssparser-macros", allow = ["MPL-2.0"] },
+    { crate = "dtoa-short", allow = ["MPL-2.0"] },
+    { crate = "selectors", allow = ["MPL-2.0"] },
 ]
 
 [bans]
@@ -44,6 +51,7 @@ skip = [
     { crate = "getrandom@0.2.17", reason = "ring 0.17 (TLS for ureq, via rustls) still depends on getrandom 0.2; tempfile uses 0.4" },
     { crate = "windows-sys@0.52.0", reason = "ring 0.17 (TLS for ureq, via rustls) still depends on windows-sys 0.52; Windows only" },
     { crate = "hashbrown@0.16.1", reason = "rsqlite-vfs, via sqlite-wasm-rs (a wasm32-only dependency of rusqlite), still depends on hashbrown 0.16" },
+    { crate = "syn@2.0.119", reason = "derive_more-impl and phf_macros (proc macros for scraper's selectors and cssparser) still depend on syn 2" },
 ]
 
 [sources]
```

Run `cargo build -p codecite-index` to update `Cargo.lock`, then `cargo deny check`.
Expected: `advisories ok, bans ok, licenses ok, sources ok`.

- [ ] **Step 2: Add the fixtures**

Each is a cut-down page in one layout's structure: front matter, a table of contents, a few nested sections, an appendix, references and an unnumbered section after them.

`crates/codecite-index/tests/fixtures/spec/rfc-v3.html` (xml2rfc v3: `<section id="section-4.3">` around an `<h3>` holding `a.section-number` and `a.section-name`, a `¶` link after each paragraph):

```html
<!DOCTYPE html>
<html lang="en" class="RFC">
<head>
<meta charset="utf-8">
<meta content="xml2rfc 3.18.2" name="generator">
<title>RFC 9449: OAuth 2.0 Demonstrating Proof of Possession (DPoP)</title>
<style>body { font-family: serif; }</style>
</head>
<body>
<table class="ears"><tbody><tr><td class="left">RFC 9449</td><td class="center">OAuth DPoP</td></tr></tbody></table>
<div id="external-metadata" class="document-information"></div>
<h1 id="title">OAuth 2.0 Demonstrating Proof of Possession (DPoP)</h1>
<section id="section-abstract">
      <h2 id="abstract"><a href="#abstract" class="selfRef">Abstract</a></h2>
<p id="section-abstract-1">This document describes a mechanism for sender-constraining OAuth 2.0 tokens.<a href="#section-abstract-1" class="pilcrow">¶</a></p>
</section>
<div id="toc">
<section id="section-toc.1">
        <h2 id="name-table-of-contents"><a href="#name-table-of-contents" class="selfRef">Table of Contents</a></h2>
<nav class="toc"><ul class="compact toc ulBare ulEmpty">
<li class="compact toc ulBare ulEmpty" id="section-toc.1-1.1"><p id="section-toc.1-1.1.1"><a href="#section-1" class="auto internal xref">1</a>.  <a href="#name-introduction" class="internal xref">Introduction</a></p></li>
</ul></nav>
</section>
</div>
<div id="introduction">
<section id="section-1">
      <h2 id="name-introduction">
<a href="#section-1" class="section-number selfRef">1. </a><a href="#name-introduction" class="section-name selfRef">Introduction</a>
      </h2>
<p id="section-1-1">DPoP is an application-level mechanism for sender-constraining OAuth access tokens
and refresh tokens.<a href="#section-1-1" class="pilcrow">¶</a></p>
</section>
</div>
<section id="section-4">
      <h2 id="name-dpop-proofs">
<a href="#section-4" class="section-number selfRef">4. </a><a href="#name-dpop-proofs" class="section-name selfRef">DPoP Proofs</a>
      </h2>
<p id="section-4-1">A DPoP proof is a JWT.<a href="#section-4-1" class="pilcrow">¶</a></p>
<div id="checking">
<section id="section-4.3">
        <h3 id="name-checking-dpop-proofs">
<a href="#section-4.3" class="section-number selfRef">4.3. </a><a href="#name-checking-dpop-proofs" class="section-name selfRef">Checking DPoP Proofs</a>
        </h3>
<p id="section-4.3-1">To validate a DPoP proof, the receiving server <span class="bcp14">MUST</span> ensure the following:<a href="#section-4.3-1" class="pilcrow">¶</a></p>
<ol start="1" type="1" class="compact type-1" id="section-4.3-2">
<li id="section-4.3-2.1">There is not more than one <code>DPoP</code> HTTP request header field.<a href="#section-4.3-2.1" class="pilcrow">¶</a>
</li>
          <li id="section-4.3-2.2">The <code>typ</code> JOSE Header Parameter has the value <code>dpop+jwt</code>.<a href="#section-4.3-2.2" class="pilcrow">¶</a>
</li>
        </ol>
<div class="lang-http-message sourcecode" id="section-4.3-3">
<pre>DPoP: eyJ0eXAiOiJkcG9wK2p3dCJ9
  .eyJqdGkiOiItQndDM0VTYzZhY2MybFRjIn0
</pre><a href="#section-4.3-3" class="pilcrow">¶</a>
</div>
<section id="section-4.3.1">
          <h4 id="name-nonces">
<a href="#section-4.3.1" class="section-number selfRef">4.3.1. </a><a href="#name-nonces" class="section-name selfRef">Nonces</a>
          </h4>
<p id="section-4.3.1-1">A server <span class="bcp14">MAY</span> supply a nonce.<a href="#section-4.3.1-1" class="pilcrow">¶</a></p>
</section>
</section>
</div>
</section>
<section id="section-13">
      <h2 id="name-references">
<a href="#section-13" class="section-number selfRef">13. </a><a href="#name-references" class="section-name selfRef">References</a>
      </h2>
<section id="section-13.1">
        <h3 id="name-normative-references">
<a href="#section-13.1" class="section-number selfRef">13.1. </a><a href="#name-normative-references" class="section-name selfRef">Normative References</a>
        </h3>
<dl class="references">
<dt id="RFC2119">[RFC2119]</dt>
        <dd><span class="refAuthor">Bradner, S.</span>, <span class="refTitle">"Key words for use in RFCs to Indicate Requirement Levels"</span>.</dd>
</dl>
</section>
</section>
<section id="appendix-A">
      <h2 id="name-document-history">
<a href="#appendix-A" class="section-number selfRef">Appendix A. </a><a href="#name-document-history" class="section-name selfRef">Document History</a>
      </h2>
<section id="appendix-A.1">
        <h3 id="name-draft-01">
<a href="#appendix-A.1" class="section-number selfRef">A.1. </a><a href="#name-draft-01" class="section-name selfRef">Draft 01</a>
        </h3>
<ul class="normal">
<li class="normal" id="appendix-A.1-1.1">Added nonces.<a href="#appendix-A.1-1.1" class="pilcrow">¶</a></li>
</ul>
</section>
</section>
<section id="appendix-B">
      <h2 id="name-acknowledgements">
<a href="#name-acknowledgements" class="section-name selfRef">Acknowledgements</a>
      </h2>
<p id="appendix-B-1">We would like to thank everyone.<a href="#appendix-B-1" class="pilcrow">¶</a></p>
</section>
<div id="authors-addresses">
<section id="appendix-C">
      <h2 id="name-authors-addresses">
<a href="#name-authors-addresses" class="section-name selfRef">Authors' Addresses</a>
      </h2>
<address class="vcard"><div dir="auto" class="left"><span class="fn nameRole">Daniel Fett</span></div></address>
</section>
</div>
<script>var toc = document.getElementById("toc");</script>
</body>
</html>
```

`crates/codecite-index/tests/fixtures/spec/rfc-text.html` (older RFC HTML: the text RFC in `<pre class="newpage">` blocks, a `span.h2`..`span.h5` with an `a.selflink` per heading, a `span.grey` page header and footer at each page break):

```html
<pre>Internet Engineering Task Force (IETF)                     D. Hardt, Ed.
Request for Comments: 6749                                     Microsoft
Obsoletes: <a href="./rfc5849">5849</a>                                             October 2012
Category: Standards Track


                 <span class="h1">The OAuth 2.0 Authorization Framework</span>

Abstract

   The OAuth 2.0 authorization framework enables a third-party
   application to obtain limited access to an HTTP service.

Table of Contents

   <a href="#section-1">1</a>. Introduction ....................................................<a href="#page-4">4</a>
   <a href="#section-4">4</a>. Obtaining Authorization ........................................<a href="#page-23">23</a>

<span class="h2"><a class="selflink" id="section-1" href="#section-1">1</a>.  Introduction</span>

   In the traditional client-server authentication model, the client
   requests an access-restricted resource.

<span class="h2"><a class="selflink" id="section-4" href="#section-4">4</a>.  Obtaining Authorization</span>

   To request an access token, the client obtains authorization from the
   resource owner.

<span class="h3"><a class="selflink" id="section-4.1" href="#section-4.1">4.1</a>.  Authorization Code Grant</span>

   The authorization code grant type is used to obtain both access
   tokens and refresh tokens.

<span class="h4"><a class="selflink" id="section-4.1.2" href="#section-4.1.2">4.1.2</a>.  Authorization Response</span>

   If the resource owner grants the access request, the authorization
   server issues an authorization code.

<span class="h5"><a class="selflink" id="section-4.1.2.1" href="#section-4.1.2.1">4.1.2.1</a>.  Error Response</span>

   If the request fails due to a missing, invalid, or mismatching
   redirection URI, the authorization server SHOULD inform the
   resource owner of the error.



<span class="grey">Hardt                        Standards Track                   [Page 26]</span></pre>
<hr class='noprint'/><!--NewPage--><pre class='newpage'><span id="page-27" ></span>
<span class="grey"><a href="./rfc6749">RFC 6749</a>                        OAuth 2.0                   October 2012</span>


   See <a href="#section-4.1.2">Section&nbsp;4.1.2</a> for the response.

<span class="h2"><a class="selflink" id="section-12" href="#section-12">12</a>.  References</span>

<span class="h3"><a class="selflink" id="section-12.1" href="#section-12.1">12.1</a>.  Normative References</span>

   [RFC2119]  Bradner, S., "Key words for use in RFCs to Indicate
              Requirement Levels", BCP 14, <a href="./rfc2119">RFC 2119</a>, March 1997.

<span class="h2"><a class="selflink" id="appendix-A" href="#appendix-A">Appendix A</a>.  Augmented Backus-Naur Form (ABNF) Syntax</span>

   This section provides Augmented Backus-Naur Form (ABNF) syntax
   descriptions for the elements defined in this specification.

<span class="h3"><a class="selflink" id="appendix-A.1" href="#appendix-A.1">A.1</a>.  "client_id" Syntax</span>

     client-id     = *VSCHAR

Author's Address

   Dick Hardt (editor)
   Microsoft

<span class="grey">Hardt                        Standards Track                   [Page 76]</span>
</pre>
```

`crates/codecite-index/tests/fixtures/spec/openid.html` (older xml2rfc: `<a name="IDTokenValidation">`, `<br /><hr />`, a `table.TOCbug` box and `<a name="rfc.section.3.1.3.7">` before each `<h3>`; §3.1.3.8 has no author anchor):

```html
<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/1999/REC-html401-19991224/loose.dtd">
<html lang="en"><head><title>Final: OpenID Connect Core 1.0 incorporating errata set 2</title>
<meta http-equiv="Content-Type" content="text/html; charset=UTF-8">
<style type='text/css'><!-- body { font-family: verdana; } --></style>
<meta name="generator" content="xml2rfc v1.37pre1 (http://xml.resource.org/)">
</head>
<body>
<table summary="layout" width="66%" border="0" cellpadding="0" cellspacing="0"><tr><td><table summary="layout" width="100%" border="0" cellpadding="2" cellspacing="1">
<tr><td class="header">Final</td><td class="header">N. Sakimura</td></tr>
</table></td></tr></table>
<div align="center"><h1><br />OpenID Connect Core 1.0 incorporating errata set 2</h1></div>

<h3>Abstract</h3>

<p>OpenID Connect 1.0 is a simple identity layer on top of the OAuth 2.0
      protocol.
</p>
<a name="toc"></a><br /><hr />
<h3>Table of Contents</h3>
<p class="toc">
<a href="#Introduction">1.</a>&nbsp;
Introduction<br />
<a href="#IDTokenValidation">3.1.3.7.</a>&nbsp;
ID Token Validation<br />
</p>
<br clear="all" />

<a name="Introduction"></a><br /><hr />
<table summary="layout" cellpadding="0" cellspacing="2" class="TOCbug" align="right"><tr><td class="TOCbug"><a href="#toc">&nbsp;TOC&nbsp;</a></td></tr></table>
<a name="rfc.section.1"></a><h3>1.&nbsp;
Introduction</h3>

<p>
	    OpenID Connect 1.0 is a simple identity layer.
</p>
<a name="Authentication"></a><br /><hr />
<table summary="layout" cellpadding="0" cellspacing="2" class="TOCbug" align="right"><tr><td class="TOCbug"><a href="#toc">&nbsp;TOC&nbsp;</a></td></tr></table>
<a name="rfc.section.3"></a><h3>3.&nbsp;
Authentication</h3>

<p>
	    OpenID Connect performs authentication to log in the End-User.
</p>
<a name="CodeFlowAuth"></a><br /><hr />
<table summary="layout" cellpadding="0" cellspacing="2" class="TOCbug" align="right"><tr><td class="TOCbug"><a href="#toc">&nbsp;TOC&nbsp;</a></td></tr></table>
<a name="rfc.section.3.1"></a><h3>3.1.&nbsp;
Authentication using the Authorization Code Flow</h3>

<p>This section describes how to perform authentication using the Authorization Code Flow.
</p>
<a name="TokenEndpoint"></a><br /><hr />
<table summary="layout" cellpadding="0" cellspacing="2" class="TOCbug" align="right"><tr><td class="TOCbug"><a href="#toc">&nbsp;TOC&nbsp;</a></td></tr></table>
<a name="rfc.section.3.1.3"></a><h3>3.1.3.&nbsp;
Token Endpoint</h3>

<p>
	    To obtain an Access Token, an ID Token, and optionally a Refresh Token,
	    the RP (Client) sends a Token Request to the Token Endpoint.
</p>
<a name="IDTokenValidation"></a><br /><hr />
<table summary="layout" cellpadding="0" cellspacing="2" class="TOCbug" align="right"><tr><td class="TOCbug"><a href="#toc">&nbsp;TOC&nbsp;</a></td></tr></table>
<a name="rfc.section.3.1.3.7"></a><h3>3.1.3.7.&nbsp;
ID Token Validation</h3>

<p>
              Clients MUST validate the ID Token in the Token Response
	      in the following manner:

</p>
<p>
	      </p>
<ol class="text">
<li>
		  If the ID Token is encrypted, decrypt it using the
		  keys and algorithms that the Client specified during Registration.

</li>
<li>
		  The Issuer Identifier for the OpenID Provider
		  MUST exactly match the value of the
		  <tt>iss</tt> (issuer) Claim.

</li>
</ol><p>

</p>
<br /><hr />
<table summary="layout" cellpadding="0" cellspacing="2" class="TOCbug" align="right"><tr><td class="TOCbug"><a href="#toc">&nbsp;TOC&nbsp;</a></td></tr></table>
<a name="rfc.section.3.1.3.8"></a><h3>3.1.3.8.&nbsp;
Access Token Validation</h3>

<p>
	      When using the Authorization Code Flow, the at_hash Claim is OPTIONAL.
</p>
<a name="rfc.references"></a><br /><hr />
<table summary="layout" cellpadding="0" cellspacing="2" class="TOCbug" align="right"><tr><td class="TOCbug"><a href="#toc">&nbsp;TOC&nbsp;</a></td></tr></table>
<a name="rfc.section.19"></a><h3>19.&nbsp;
References</h3>

<a name="rfc.references1"></a><br /><hr />
<table summary="layout" cellpadding="0" cellspacing="2" class="TOCbug" align="right"><tr><td class="TOCbug"><a href="#toc">&nbsp;TOC&nbsp;</a></td></tr></table>
<h3>19.1.&nbsp;Normative References</h3>
<table width="99%" border="0">
<tr><td class="author-text" valign="top"><a name="RFC2119">[RFC2119]</a></td>
<td class="author-text">Bradner, S., &ldquo;Key words for use in RFCs to Indicate Requirement Levels&rdquo;.</td></tr>
</table>

<a name="AuthorizationExamples"></a><br /><hr />
<table summary="layout" cellpadding="0" cellspacing="2" class="TOCbug" align="right"><tr><td class="TOCbug"><a href="#toc">&nbsp;TOC&nbsp;</a></td></tr></table>
<a name="rfc.section.A"></a><h3>Appendix A.&nbsp;
Authorization Examples</h3>

<p>
	The following are non-normative examples.
</p>
<a name="rfc.authors"></a><br /><hr />
<table summary="layout" cellpadding="0" cellspacing="2" class="TOCbug" align="right"><tr><td class="TOCbug"><a href="#toc">&nbsp;TOC&nbsp;</a></td></tr></table>
<h3>Authors' Addresses</h3>
<table summary="layout" width="99%" border="0" cellpadding="0" cellspacing="0">
<tr><td class="author-text">&nbsp;</td><td class="author-text">Nat Sakimura</td></tr>
</table>
</body></html>
```

- [ ] **Step 3: Write the failing tests**

Create `crates/codecite-index/src/spec.rs` holding the module doc comment, `mod render;`, the `use` lines and the public types from Step 4, and this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Each section as (number, title, anchor, parent number).
    fn outline(doc: &SpecDocument) -> Vec<(&str, &str, &str, Option<&str>)> {
        doc.sections
            .iter()
            .map(|s| {
                let parent = s.parent.map(|p| doc.sections[p].number.as_str());
                (
                    s.number.as_str(),
                    s.title.as_str(),
                    s.anchor.as_str(),
                    parent,
                )
            })
            .collect()
    }

    /// Every section's text is its own line range of the document text.
    fn assert_ranges_match(doc: &SpecDocument) {
        let lines: Vec<&str> = doc.text.lines().collect();
        for s in &doc.sections {
            let range = &lines[s.start_line as usize - 1..s.end_line as usize];
            assert_eq!(range.join("\n"), s.text, "§{}", s.number);
        }
    }

    #[test]
    fn rfc_v3_fixture() {
        let doc = split_spec(include_str!("../tests/fixtures/spec/rfc-v3.html")).unwrap();
        assert_eq!(doc.format, SpecFormat::RfcV3);
        assert_eq!(
            outline(&doc),
            [
                ("1", "Introduction", "section-1", None),
                ("4", "DPoP Proofs", "section-4", None),
                ("4.3", "Checking DPoP Proofs", "section-4.3", Some("4")),
                ("4.3.1", "Nonces", "section-4.3.1", Some("4.3")),
                ("A", "Document History", "appendix-A", None),
                ("A.1", "Draft 01", "appendix-A.1", Some("A")),
            ]
        );
        assert_ranges_match(&doc);
        assert_eq!(
            doc.section("4.3").unwrap().text,
            "4.3. Checking DPoP Proofs\n\
             \n\
             To validate a DPoP proof, the receiving server MUST ensure the following:\n\
             \n\
             1. There is not more than one DPoP HTTP request header field.\n\
             2. The typ JOSE Header Parameter has the value dpop+jwt.\n\
             \n\
             DPoP: eyJ0eXAiOiJkcG9wK2p3dCJ9\n  .eyJqdGkiOiItQndDM0VTYzZhY2MybFRjIn0"
        );
    }

    #[test]
    fn rfc_text_fixture() {
        let doc = split_spec(include_str!("../tests/fixtures/spec/rfc-text.html")).unwrap();
        assert_eq!(doc.format, SpecFormat::RfcText);
        assert_eq!(
            outline(&doc),
            [
                ("1", "Introduction", "section-1", None),
                ("4", "Obtaining Authorization", "section-4", None),
                ("4.1", "Authorization Code Grant", "section-4.1", Some("4")),
                (
                    "4.1.2",
                    "Authorization Response",
                    "section-4.1.2",
                    Some("4.1")
                ),
                (
                    "4.1.2.1",
                    "Error Response",
                    "section-4.1.2.1",
                    Some("4.1.2")
                ),
                (
                    "A",
                    "Augmented Backus-Naur Form (ABNF) Syntax",
                    "appendix-A",
                    None
                ),
                ("A.1", "\"client_id\" Syntax", "appendix-A.1", Some("A")),
            ]
        );
        assert_ranges_match(&doc);
        // The page break inside §4.1.2.1 is gone: no footer, no header and
        // a single blank line where the page ended.
        assert_eq!(
            doc.section("4.1.2.1").unwrap().text,
            "4.1.2.1.  Error Response\n\
             \n\
             \x20  If the request fails due to a missing, invalid, or mismatching\n\
             \x20  redirection URI, the authorization server SHOULD inform the\n\
             \x20  resource owner of the error.\n\
             \n\
             \x20  See Section 4.1.2 for the response."
        );
        // "Author's Address" is an unnumbered heading: it ends Appendix A.1.
        assert_eq!(
            doc.section("A.1").unwrap().text,
            "A.1.  \"client_id\" Syntax\n\n     client-id     = *VSCHAR"
        );
    }

    #[test]
    fn openid_fixture() {
        let doc = split_spec(include_str!("../tests/fixtures/spec/openid.html")).unwrap();
        assert_eq!(doc.format, SpecFormat::OpenId);
        assert_eq!(
            outline(&doc),
            [
                ("1", "Introduction", "Introduction", None),
                ("3", "Authentication", "Authentication", None),
                (
                    "3.1",
                    "Authentication using the Authorization Code Flow",
                    "CodeFlowAuth",
                    Some("3")
                ),
                ("3.1.3", "Token Endpoint", "TokenEndpoint", Some("3.1")),
                (
                    "3.1.3.7",
                    "ID Token Validation",
                    "IDTokenValidation",
                    Some("3.1.3")
                ),
                (
                    "3.1.3.8",
                    "Access Token Validation",
                    "rfc.section.3.1.3.8",
                    Some("3.1.3")
                ),
                ("A", "Authorization Examples", "AuthorizationExamples", None),
            ]
        );
        assert_ranges_match(&doc);
        assert_eq!(
            doc.section("3.1.3.7").unwrap().text,
            "3.1.3.7. ID Token Validation\n\
             \n\
             Clients MUST validate the ID Token in the Token Response in the following manner:\n\
             \n\
             1. If the ID Token is encrypted, decrypt it using the keys and algorithms that the Client specified during Registration.\n\
             2. The Issuer Identifier for the OpenID Provider MUST exactly match the value of the iss (issuer) Claim."
        );
    }

    #[test]
    fn front_matter_toc_and_references_are_not_sections() {
        for html in [
            include_str!("../tests/fixtures/spec/rfc-v3.html"),
            include_str!("../tests/fixtures/spec/rfc-text.html"),
            include_str!("../tests/fixtures/spec/openid.html"),
        ] {
            let doc = split_spec(html).unwrap();
            for s in &doc.sections {
                assert!(!s.title.contains("References"), "{s:?}");
                assert!(!s.text.contains("Table of Contents"), "{s:?}");
                assert!(!s.text.contains("Abstract"), "{s:?}");
                assert!(!s.text.contains("Address"), "{s:?}");
            }
            // The text before the first section is still in the rendering.
            assert!(doc.text.contains("Abstract"));
        }
    }

    #[test]
    fn unknown_pages_are_an_error() {
        assert_eq!(
            split_spec("<html><body><h1>Hello</h1><p>1. Not a spec</p></body></html>"),
            Err(SpecError::UnknownFormat)
        );
        assert_eq!(split_spec(""), Err(SpecError::UnknownFormat));
    }

    #[test]
    fn malformed_html_still_splits() {
        // Unclosed tags and stray end tags: html5ever repairs them.
        let html = "<pre><span class=\"h2\"><a class=\"selflink\" id=\"section-1\">1</a>.  Intro\
                    </span>\n   Text <b>bold\n</i></pre><p>";
        let doc = split_spec(html).unwrap();
        assert_eq!(outline(&doc), [("1", "Intro", "section-1", None)]);
        assert_eq!(doc.sections[0].text, "1.  Intro\n   Text bold");
    }

    /// A real spec page, fetched through the pinned cache in the temp
    /// directory, so only the first run downloads it.
    fn fetch_real(name: &str, url: &str, sha256: &str) -> String {
        use crate::{DocumentSource, Fetcher, LicenseClass};
        let doc = DocumentSource {
            name: name.into(),
            url: url.into(),
            sha256: sha256.into(),
            license: None,
            license_class: LicenseClass::Quote,
            current: true,
            aliases: Vec::new(),
            obsoleted_by: None,
            updated_by: None,
        };
        let fetcher = Fetcher::new(std::env::temp_dir().join("codecite-documents"));
        String::from_utf8(fetcher.fetch_pinned(&doc).unwrap().bytes).unwrap()
    }

    #[test]
    #[ignore = "downloads RFC 9449; run with `cargo test -- --ignored`"]
    fn real_rfc9449() {
        let html = fetch_real(
            "rfc9449",
            "https://www.rfc-editor.org/rfc/rfc9449.html",
            "2d8657f072913d55627dc25c02d3ad305bd62bed7f06c78c7d0621768b234784",
        );
        let doc = split_spec(&html).unwrap();
        assert_eq!(doc.format, SpecFormat::RfcV3);
        assert_ranges_match(&doc);
        let s = doc.section("4.3").unwrap();
        assert_eq!(
            (s.title.as_str(), s.anchor.as_str()),
            ("Checking DPoP Proofs", "section-4.3")
        );
        assert_eq!(doc.sections[s.parent.unwrap()].number, "4");
        assert!(
            s.text.contains("12. If presented to a protected resource"),
            "{}",
            s.text
        );
        assert!(doc.section("13").is_none(), "References");
        assert!(doc.sections.iter().all(|s| s.number != "13.1"));
        println!("RFC 9449: {} sections", doc.sections.len());
    }

    #[test]
    #[ignore = "downloads RFC 6749; run with `cargo test -- --ignored`"]
    fn real_rfc6749() {
        let html = fetch_real(
            "rfc6749",
            "https://www.rfc-editor.org/rfc/rfc6749.html",
            "535362fa3b4ca668d4734c244c6ed8c811f2ccd7ea1612a4201e5d8922a74b4e",
        );
        let doc = split_spec(&html).unwrap();
        assert_eq!(doc.format, SpecFormat::RfcText);
        assert_ranges_match(&doc);
        let s = doc.section("4.1.2.1").unwrap();
        assert_eq!(
            (s.title.as_str(), s.anchor.as_str()),
            ("Error Response", "section-4.1.2.1")
        );
        assert_eq!(doc.sections[s.parent.unwrap()].number, "4.1.2");
        assert!(!doc.text.contains("[Page "), "page footers are dropped");
        assert!(doc.section("12").is_none(), "References");
        assert!(!doc.sections.last().unwrap().text.contains("Hardt (editor)"));
        println!("RFC 6749: {} sections", doc.sections.len());
    }

    #[test]
    #[ignore = "downloads OpenID Connect Core 1.0; run with `cargo test -- --ignored`"]
    fn real_openid_connect_core() {
        let html = fetch_real(
            "openid-connect-core-1_0",
            "https://openid.net/specs/openid-connect-core-1_0.html",
            "4d016752a645e8a3c259baa4173b5a90cb6c82898ce868fc8444adde262a5aed",
        );
        let doc = split_spec(&html).unwrap();
        assert_eq!(doc.format, SpecFormat::OpenId);
        assert_ranges_match(&doc);
        let s = doc.section("3.1.3.7").unwrap();
        assert_eq!(
            (s.title.as_str(), s.anchor.as_str()),
            ("ID Token Validation", "IDTokenValidation")
        );
        assert_eq!(doc.sections[s.parent.unwrap()].number, "3.1.3");
        assert!(
            s.text.contains("13. If the auth_time Claim was requested,"),
            "{}",
            s.text
        );
        assert!(!doc.text.contains("TOC"), "the TOC link boxes are dropped");
        assert!(doc.section("19").is_none(), "References");
        println!("Core: {} sections", doc.sections.len());
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        // The walk is iterative, so 5,000 nested elements fit in a 256 KiB
        // stack, where a recursive walk would overflow. The depth stays low
        // because html5ever's parse is quadratic in nesting depth.
        let depth = 5_000;
        let html = format!(
            "<section id=\"section-1\"><h2><a class=\"section-number\">1. </a>\
             <a class=\"section-name\">Deep</a></h2>{}x{}</section>",
            "<div>".repeat(depth),
            "</div>".repeat(depth)
        );
        let doc = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || split_spec(&html))
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        assert_eq!(doc.sections.len(), 1);
        assert!(doc.sections[0].text.ends_with('x'));
    }
}
```

Create `crates/codecite-index/src/spec/render.rs` with its `use` lines and this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_numbers() {
        for good in ["1", "4.3", "3.1.3.7", "12", "A", "A.1", "B.10"] {
            assert!(is_section_number(good), "{good}");
        }
        for bad in [
            "",
            "abstract",
            "boilerplate.1",
            "toc.1",
            "4.",
            ".4",
            "4..3",
            "AB",
            "a",
            "4.a",
        ] {
            assert!(!is_section_number(bad), "{bad}");
        }
        assert_eq!(section_number("section-4.3"), Some("4.3"));
        assert_eq!(section_number("appendix-A.1"), Some("A.1"));
        assert_eq!(section_number("section-abstract"), None);
        assert_eq!(section_number("name-introduction"), None);
    }

    #[test]
    fn page_footers_outside_grey_spans_are_dropped() {
        let html = Html::parse_document(
            "<pre><span class=\"h2\"><a class=\"selflink\" id=\"section-1\">1</a>.  Intro</span>\n\
             \n   Text.\n\nHardt     Standards Track     [Page 76]\n\n   See [Page 2].\n</pre>",
        );
        let rendered = render(&html, SpecFormat::RfcText);
        assert_eq!(
            rendered.lines,
            ["1.  Intro", "", "   Text.", "", "   See [Page 2]."]
        );
        assert_eq!(rendered.headings[0].line, 0);
        assert!(is_page_footer(
            "Hardt (editor)    Standards Track    [Page 3]"
        ));
        assert!(!is_page_footer("   indented text [Page 3]"));
        assert!(!is_page_footer("Hardt [Page ]"));
        assert!(!is_page_footer("Hardt [Page 3] and more"));
    }

    #[test]
    fn collapse_handles_no_break_spaces() {
        assert_eq!(
            collapse("3.1.3.7.\u{a0}\n  ID Token\tValidation "),
            "3.1.3.7. ID Token Validation"
        );
    }

    #[test]
    fn lists_are_numbered_from_their_start() {
        let html = Html::parse_document(
            "<section id=\"section-1\"><h2><a class=\"section-number\">1. </a>Lists</h2>\
             <ol start=\"3\"><li>three</li><li>four</li></ol>\
             <ol type=\"a\"><li>first</li><li>second</li></ol>\
             <ul><li>dot</li></ul></section>",
        );
        let lines = render(&html, SpecFormat::RfcV3).lines;
        assert_eq!(
            lines,
            [
                "1. Lists",
                "",
                "3. three",
                "4. four",
                "",
                "a. first",
                "b. second",
                "",
                "- dot"
            ]
        );
    }
}
```

Wire the module into `lib.rs`:

```diff
diff --git a/crates/codecite-index/src/lib.rs b/crates/codecite-index/src/lib.rs
index 055750f..8ea0455 100644
--- a/crates/codecite-index/src/lib.rs
+++ b/crates/codecite-index/src/lib.rs
@@ -33,6 +33,9 @@ mod test_repo;
 #[cfg(test)]
 mod test_server;
 
+mod spec;
+pub use spec::{SpecDocument, SpecError, SpecFormat, SpecSection, split_spec};
+
 mod walk;
 pub use walk::{
     DEFAULT_MAX_FILE_SIZE, LocalFile, LocalTree, SkipReason, SkippedFile, WalkError, WalkOptions,
```

Run: `cargo test -p codecite-index --lib spec::`
Expected: FAIL to compile (`split_spec`, `render`, `is_section_number` not found).

- [ ] **Step 4: Render the page to lines and headings**

The whole of `render.rs` above its test module:

```rust
//! Renders a parsed spec page as plain-text lines and finds its headings,
//! in one walk over the HTML tree.

use ego_tree::NodeId;
use ego_tree::iter::Edge;
use scraper::{CaseSensitivity, ElementRef, Html, Node, Selector};

use super::SpecFormat;

/// The page as lines, and every heading in it in document order.
pub(super) struct Rendered {
    /// Lines without line endings. Blank lines are empty strings, and two
    /// never follow each other.
    pub(super) lines: Vec<String>,
    pub(super) headings: Vec<Heading>,
}

/// A heading: the start of a section, or of an unnumbered part such as
/// the table of contents, which ends the section before it.
pub(super) struct Heading {
    /// The heading's line in [`Rendered::lines`], counting from 0.
    pub(super) line: usize,
    /// The number, title and anchor, for a numbered section.
    pub(super) numbered: Option<Numbered>,
}

pub(super) struct Numbered {
    pub(super) number: String,
    pub(super) title: String,
    pub(super) anchor: String,
}

/// Which layout the page uses, from the markup each one puts around its
/// section numbers.
pub(super) fn detect(html: &Html) -> Option<SpecFormat> {
    let has = |css: &str| {
        let selector = Selector::parse(css).expect("a valid selector");
        html.select(&selector).next().is_some()
    };
    if has("section > :is(h1, h2, h3, h4, h5, h6) > a.section-number") {
        Some(SpecFormat::RfcV3)
    } else if has("a[name^=\"rfc.section.\"]") {
        Some(SpecFormat::OpenId)
    } else if has("pre span a.selflink") {
        Some(SpecFormat::RfcText)
    } else {
        None
    }
}

/// Walks the tree once, writing text and recording headings as they come.
///
/// `traverse` yields an `Open` edge when the walk enters a node and a
/// `Close` edge when it leaves, so this needs no recursion and a deeply
/// nested page can't overflow the stack.
pub(super) fn render(html: &Html, format: SpecFormat) -> Rendered {
    let mut out = Writer::default();
    let mut headings = Vec::new();
    let mut lists: Vec<List> = Vec::new();
    let mut pre_depth = 0usize;
    let mut skipping: Option<NodeId> = None;

    for edge in html.tree.root().traverse() {
        match edge {
            Edge::Open(node) => {
                if skipping.is_some() {
                    continue;
                }
                match node.value() {
                    Node::Text(text) if pre_depth > 0 => out.pre_text(text),
                    Node::Text(text) => out.text(text),
                    Node::Element(_) => {
                        let el = ElementRef::wrap(node).expect("an element node");
                        if is_skipped(el) {
                            skipping = Some(node.id());
                            continue;
                        }
                        open(el, &mut out, &mut lists, &mut pre_depth);
                        if let Some(numbered) = heading(format, el) {
                            headings.push(Heading {
                                line: out.next_line(),
                                numbered,
                            });
                        }
                    }
                    _ => {}
                }
            }
            Edge::Close(node) => {
                if skipping == Some(node.id()) {
                    skipping = None;
                } else if skipping.is_none()
                    && let Some(el) = ElementRef::wrap(node)
                {
                    close(el, &mut out, &mut lists, &mut pre_depth);
                }
            }
        }
    }
    let mut lines = out.finish();
    if format == SpecFormat::RfcText {
        lines = drop_page_footers(lines, &mut headings);
        add_plain_headings(&lines, &mut headings);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    Rendered { lines, headings }
}

/// Elements whose text is not part of the spec: page furniture, scripts,
/// and the links each layout adds beside its paragraphs and headings.
fn is_skipped(el: ElementRef) -> bool {
    let e = el.value();
    let class = |name| e.has_class(name, CaseSensitivity::AsciiCaseInsensitive);
    match e.name() {
        "head" | "script" | "style" | "noscript" | "template" => true,
        // The ¶ after each paragraph of an xml2rfc v3 page.
        "a" => class("pilcrow"),
        // The "TOC" link box before each heading of an OpenID page.
        "table" => class("TOCbug"),
        // The page header and footer at each page break of an older RFC.
        "span" => class("grey"),
        _ => false,
    }
}

/// A list being written, for numbering its items.
enum List {
    /// `<ol>`: the next item's number and whether it is written as a letter.
    Ordered { next: u32, letters: Option<u8> },
    /// `<ul>`.
    Unordered,
}

fn open(el: ElementRef, out: &mut Writer, lists: &mut Vec<List>, pre_depth: &mut usize) {
    let e = el.value();
    match e.name() {
        "pre" => {
            out.end_paragraph();
            *pre_depth += 1;
        }
        "ol" => {
            out.end_paragraph();
            let next = e.attr("start").and_then(|s| s.parse().ok()).unwrap_or(1);
            let letters = match e.attr("type") {
                Some("a") => Some(b'a'),
                Some("A") => Some(b'A'),
                _ => None,
            };
            lists.push(List::Ordered { next, letters });
        }
        "ul" => {
            out.end_paragraph();
            lists.push(List::Unordered);
        }
        "li" => {
            out.end_line();
            match lists.last_mut() {
                Some(List::Ordered { next, letters }) => {
                    let label = match *letters {
                        Some(base) if (1..=26).contains(next) => {
                            char::from(base + (*next - 1) as u8).to_string()
                        }
                        _ => next.to_string(),
                    };
                    out.label = Some(format!("{label}. "));
                    *next += 1;
                }
                Some(List::Unordered) | None => out.label = Some("- ".to_owned()),
            }
        }
        "br" => out.break_line(pre_depth),
        "dt" | "dd" | "tr" => out.end_line(),
        "td" | "th" => out.text(" "),
        name if is_block(name) => out.end_paragraph(),
        _ => {}
    }
}

fn close(el: ElementRef, out: &mut Writer, lists: &mut Vec<List>, pre_depth: &mut usize) {
    match el.value().name() {
        "pre" => {
            *pre_depth = pre_depth.saturating_sub(1);
            out.end_paragraph();
        }
        "ol" | "ul" => {
            lists.pop();
            out.end_paragraph();
        }
        "li" | "dt" | "dd" | "tr" => out.end_line(),
        name if is_block(name) => out.end_paragraph(),
        _ => {}
    }
}

/// Elements that start and end a paragraph.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "section"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "dl"
            | "table"
            | "blockquote"
            | "figure"
            | "figcaption"
            | "address"
            | "aside"
            | "nav"
            | "header"
            | "footer"
            | "hr"
            | "center"
    )
}

/// `Some` when `el` is a heading in this layout, holding its number, title
/// and anchor when it starts a numbered section.
fn heading(format: SpecFormat, el: ElementRef) -> Option<Option<Numbered>> {
    let e = el.value();
    let is_h = matches!(e.name(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
    match format {
        // <section id="section-4.3"><h3><a class="section-number">4.3. </a>
        //   <a class="section-name">Checking DPoP Proofs</a></h3>
        SpecFormat::RfcV3 if is_h => Some(rfc_v3_heading(el)),
        // <span class="h3"><a class="selflink" id="section-1.2">1.2</a>.  Protocol Flow</span>
        SpecFormat::RfcText
            if e.name() == "span"
                && ["h1", "h2", "h3", "h4", "h5", "h6"]
                    .iter()
                    .any(|h| e.has_class(h, CaseSensitivity::CaseSensitive)) =>
        {
            Some(rfc_text_heading(el))
        }
        // <a name="IDTokenValidation"></a><br /><hr />
        // <a name="rfc.section.3.1.3.7"></a><h3>3.1.3.7.&nbsp;ID Token Validation</h3>
        SpecFormat::OpenId if is_h => Some(openid_heading(el)),
        _ => None,
    }
}

fn rfc_v3_heading(el: ElementRef) -> Option<Numbered> {
    let section = el.parent().and_then(ElementRef::wrap)?;
    let anchor = section.value().id()?;
    let number = section_number(anchor)?;
    let child = |class| {
        el.child_elements()
            .find(|c| c.value().has_class(class, CaseSensitivity::CaseSensitive))
    };
    child("section-number")?;
    let title = match child("section-name") {
        Some(name) => collapse(&name.text().collect::<String>()),
        None => title_after(el, number),
    };
    Some(Numbered {
        number: number.to_owned(),
        title,
        anchor: anchor.to_owned(),
    })
}

fn rfc_text_heading(el: ElementRef) -> Option<Numbered> {
    let link = el.child_elements().find(|c| {
        c.value()
            .has_class("selflink", CaseSensitivity::CaseSensitive)
    })?;
    let anchor = link.value().id()?;
    let number = section_number(anchor)?;
    Some(Numbered {
        number: number.to_owned(),
        title: title_after(el, number),
        anchor: anchor.to_owned(),
    })
}

fn openid_heading(el: ElementRef) -> Option<Numbered> {
    let mut before = el.prev_siblings().filter_map(ElementRef::wrap);
    let marker = before.next()?;
    let number = marker.value().attr("name")?.strip_prefix("rfc.section.")?;
    if !is_section_number(number) {
        return None;
    }
    // The author's anchor sits before the <br /><hr /> that precedes the
    // TOC box and the rfc.section marker. Sections without one have only
    // the marker.
    let anchor = before
        .find(|e| !matches!(e.value().name(), "br" | "hr" | "table"))
        .filter(|e| e.value().name() == "a")
        .and_then(|e| e.value().attr("name"))
        .filter(|name| !name.starts_with("rfc."))
        .map_or_else(|| format!("rfc.section.{number}"), str::to_owned);
    Some(Numbered {
        number: number.to_owned(),
        title: title_after(el, number),
        anchor,
    })
}

/// The section number in an RFC anchor: `4.3` in `section-4.3`, `A.1` in
/// `appendix-A.1`. `None` for anchors such as `section-abstract`.
fn section_number(anchor: &str) -> Option<&str> {
    let number = anchor
        .strip_prefix("section-")
        .or_else(|| anchor.strip_prefix("appendix-"))?;
    is_section_number(number).then_some(number)
}

/// `4`, `4.3.1`, `A` or `A.1`: digits, or one capital letter, then
/// dot-separated digits.
fn is_section_number(number: &str) -> bool {
    let mut parts = number.split('.');
    let first = parts.next().unwrap_or_default();
    let first_ok = (!first.is_empty() && first.bytes().all(|b| b.is_ascii_digit()))
        || (first.len() == 1 && first.bytes().all(|b| b.is_ascii_uppercase()));
    first_ok && parts.all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The heading's text after `Appendix`, the number and its dot.
fn title_after(el: ElementRef, number: &str) -> String {
    let text = collapse(&el.text().collect::<String>());
    let rest = text.strip_prefix("Appendix ").unwrap_or(&text);
    match rest.strip_prefix(number) {
        Some(rest) => rest.trim_start_matches('.').trim().to_owned(),
        None => text,
    }
}

/// `text` with each run of whitespace, no-break spaces included, as one
/// space, and none at either end.
fn collapse(text: &str) -> String {
    text.split(|c: char| c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Drops the page footers an older RFC writes outside a `grey` span, as
/// rfc-editor.org does on the last page: an unindented line ending in
/// `[Page 76]`. A blank line left after another blank line goes too, and
/// each heading moves to its line's new index.
fn drop_page_footers(lines: Vec<String>, headings: &mut [Heading]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    let mut new_index = Vec::with_capacity(lines.len());
    for line in lines {
        new_index.push(kept.len());
        let blank_after_blank = line.is_empty() && kept.last().is_none_or(String::is_empty);
        if !is_page_footer(&line) && !blank_after_blank {
            kept.push(line);
        }
    }
    for heading in headings {
        heading.line = new_index.get(heading.line).copied().unwrap_or(kept.len());
    }
    kept
}

/// `Hardt   Standards Track   [Page 76]`: unindented, ending in a page
/// number in brackets.
fn is_page_footer(line: &str) -> bool {
    let unindented = line.starts_with(|c: char| !c.is_whitespace());
    let page = line
        .rsplit_once("[Page ")
        .and_then(|(_, rest)| rest.strip_suffix(']'));
    unindented && page.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// In an older RFC an unnumbered heading is a line that starts in the
/// first column, as "Author's Address" does; body text is indented. Each
/// such line after the first section heading ends the section before it.
fn add_plain_headings(lines: &[String], headings: &mut Vec<Heading>) {
    let Some(first) = headings
        .iter()
        .find(|h| h.numbered.is_some())
        .map(|h| h.line)
    else {
        return;
    };
    let mut known: Vec<usize> = headings.iter().map(|h| h.line).collect();
    for (n, line) in lines.iter().enumerate().skip(first + 1) {
        let starts_unindented = line.chars().next().is_some_and(|c| !c.is_whitespace());
        if starts_unindented && !known.contains(&n) {
            headings.push(Heading {
                line: n,
                numbered: None,
            });
            known.push(n);
        }
    }
    headings.sort_by_key(|h| h.line);
}

/// Builds the lines: collapses whitespace outside `<pre>`, keeps it inside,
/// and never writes two blank lines in a row.
#[derive(Default)]
struct Writer {
    lines: Vec<String>,
    line: String,
    /// A list item's `3. ` or `- `, written before the item's first text,
    /// so it stays on that text's line when a `<p>` inside the item starts
    /// a new paragraph.
    label: Option<String>,
}

impl Writer {
    /// Text outside `<pre>`: each run of whitespace becomes one space, and
    /// a line never starts with one.
    fn text(&mut self, text: &str) {
        for c in text.chars() {
            if c.is_whitespace() {
                if !self.line.is_empty() && !self.line.ends_with(' ') {
                    self.line.push(' ');
                }
            } else {
                self.write_label();
                self.line.push(c);
            }
        }
    }

    fn write_label(&mut self) {
        if let Some(label) = self.label.take() {
            self.line.push_str(&label);
        }
    }

    /// Text inside `<pre>`, kept as written apart from no-break spaces.
    fn pre_text(&mut self, text: &str) {
        let mut segments = text.split('\n');
        if let Some(first) = segments.next() {
            self.push_str_pre(first);
        }
        for segment in segments {
            self.push_line();
            self.push_str_pre(segment);
        }
    }

    fn push_str_pre(&mut self, segment: &str) {
        if !segment.trim().is_empty() {
            self.write_label();
        }
        self.line
            .extend(segment.chars().map(|c| if c == '\u{a0}' { ' ' } else { c }));
    }

    /// `<br>`: a new line, which inside `<pre>` may be blank.
    fn break_line(&mut self, pre_depth: &usize) {
        if *pre_depth > 0 {
            self.push_line();
        } else {
            self.end_line();
        }
    }

    /// Ends the current line, if it has any text.
    fn end_line(&mut self) {
        if !self.line.trim().is_empty() {
            self.push_line();
        } else {
            self.line.clear();
        }
    }

    /// Ends the current line and leaves a blank line after it.
    fn end_paragraph(&mut self) {
        self.end_line();
        self.push_line();
    }

    /// The index of the line the next text lands on.
    fn next_line(&self) -> usize {
        self.lines.len()
    }

    /// Writes the current line, trimmed at the end. A blank line is
    /// dropped at the start and after another blank line.
    fn push_line(&mut self) {
        let line = std::mem::take(&mut self.line);
        let line = line.trim_end();
        if line.is_empty() && self.lines.last().is_none_or(String::is_empty) {
            return;
        }
        self.lines.push(line.to_owned());
    }

    fn finish(mut self) -> Vec<String> {
        self.end_line();
        self.lines
    }
}
```

> **Learn: parsing HTML with `scraper`.** `Html::parse_document` runs html5ever, the HTML5 parser from Servo, which repairs malformed markup the way a browser does (closes unclosed tags, drops stray end tags), so it never fails. The result is an `ego_tree::Tree<Node>`: `Node::Element` and `Node::Text` are the variants that matter here. `ElementRef::wrap(node)` turns a tree node into an element handle with `.value().name()`, `.attr("id")`, `.has_class(...)`, `.text()` (every text node below it) and sibling and child iterators. `Selector::parse` compiles a CSS selector once; `detect` uses one per layout, since checking "is there any `a.section-number` inside a heading inside a `section`" is exactly what CSS is for.

> **Learn: walking a tree without recursion.** A recursive walk uses a stack frame per level, so a page nested deeply enough overflows the thread's stack (2 MiB by default for test threads). `node.traverse()` yields `Edge::Open(node)` on the way down and `Edge::Close(node)` on the way up, in document order, from an iterator that keeps its place in the tree itself. Opening a `<pre>` bumps `pre_depth` and closing it lowers it; skipping a subtree is remembering its `NodeId` until its `Close` edge comes. That is everything recursion gave, with the state in plain variables.

Run: `cargo test -p codecite-index --lib spec::render`
Expected: the `render` tests PASS (the `spec` tests still fail to compile until Step 5).

- [ ] **Step 5: Split the lines into sections**

The whole of `spec.rs` above its test module:

```rust
//! Spec documents: splits RFC and OpenID HTML into numbered sections over a
//! plain-text rendering of the page. This module only parses: the caller
//! fetches the bytes (see [`Fetcher`](crate::Fetcher)) and passes the HTML in.

mod render;

use scraper::Html;

/// The HTML layouts [`split_spec`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpecFormat {
    /// xml2rfc v3 RFC HTML (RFC 8650 and later): one `<section>` per section.
    RfcV3,
    /// Older RFC HTML from rfc-editor.org: the plain-text RFC inside `<pre>`
    /// blocks, with a linked `<span class="h2">` per heading.
    RfcText,
    /// Pages generated by older xml2rfc versions, as the OpenID Foundation
    /// publishes its specs: an `<a name="rfc.section.N">` before each heading.
    OpenId,
}

/// One numbered section of a spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecSection {
    /// The section number as the document writes it: `3.1.3.7`, `A.1`.
    pub number: String,
    /// The heading text after the number: `ID Token Validation`.
    pub title: String,
    /// The document's own anchor for the section, without the `#`:
    /// `section-4.3` in an RFC, `IDTokenValidation` in an OpenID spec.
    pub anchor: String,
    /// The index in [`SpecDocument::sections`] of the enclosing section:
    /// §3.1.3 for §3.1.3.7. `None` for a top-level section.
    pub parent: Option<usize>,
    /// The heading's line in [`SpecDocument::text`], counting from 1.
    pub start_line: u32,
    /// The section's last line, inclusive. A section runs to the line
    /// before the next heading, so it holds its own text and not its
    /// subsections'.
    pub end_line: u32,
    /// Lines `start_line..=end_line` of [`SpecDocument::text`].
    pub text: String,
}

/// A spec page split into sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecDocument {
    /// The layout the page was read as.
    pub format: SpecFormat,
    /// The page as plain text: one line per paragraph, list item or table
    /// row, `<pre>` blocks kept as they are, blank lines between blocks.
    pub text: String,
    /// Every numbered section in document order. Front matter, the table
    /// of contents, unnumbered sections and the references are not here.
    pub sections: Vec<SpecSection>,
}

impl SpecDocument {
    /// The first section numbered `number`, e.g. `"4.1.2.1"`.
    pub fn section(&self, number: &str) -> Option<&SpecSection> {
        self.sections.iter().find(|s| s.number == number)
    }
}

/// Why a page could not be split into sections.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SpecError {
    /// The page has none of the heading markup of the three layouts.
    #[error("no RFC or OpenID section headings found; is this an RFC or OpenID spec page?")]
    UnknownFormat,
}

/// Splits a spec page's HTML into its numbered sections.
///
/// Parsing never fails on malformed HTML: `html5ever` repairs it the way a
/// browser does. The only error is a page in none of the [`SpecFormat`]s.
pub fn split_spec(html: &str) -> Result<SpecDocument, SpecError> {
    let html = Html::parse_document(html);
    let format = render::detect(&html).ok_or(SpecError::UnknownFormat)?;
    let rendered = render::render(&html, format);
    Ok(split(format, rendered))
}

/// Section titles that start the references, which are not sections.
const REFERENCES: [&str; 3] = [
    "References",
    "Normative References",
    "Informative References",
];

/// Turns the rendered lines and headings into sections. Each heading ends
/// the section before it, numbered or not, so an unnumbered heading such as
/// "Authors' Addresses" keeps its text out of the last section.
fn split(format: SpecFormat, rendered: render::Rendered) -> SpecDocument {
    let render::Rendered { lines, headings } = rendered;
    let mut sections: Vec<SpecSection> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();
    for (i, heading) in headings.iter().enumerate() {
        let Some(numbered) = &heading.numbered else {
            continue;
        };
        let number = &numbered.number;
        let inside_excluded = excluded.iter().any(|e| {
            number
                .strip_prefix(e.as_str())
                .is_some_and(|rest| rest.starts_with('.'))
        });
        if inside_excluded || REFERENCES.contains(&numbered.title.as_str()) {
            excluded.push(number.clone());
            continue;
        }
        let start = heading.line;
        let next = headings.get(i + 1).map_or(lines.len(), |h| h.line);
        let mut end = next.max(start + 1);
        while end > start + 1 && lines[end - 1].is_empty() {
            end -= 1;
        }
        let parent = number
            .rsplit_once('.')
            .and_then(|(up, _)| sections.iter().rposition(|s| s.number == up));
        sections.push(SpecSection {
            number: number.clone(),
            title: numbered.title.clone(),
            anchor: numbered.anchor.clone(),
            parent,
            start_line: line_number(start + 1),
            end_line: line_number(end),
            text: lines[start..end].join("\n"),
        });
    }
    let mut text = lines.join("\n");
    text.push('\n');
    SpecDocument {
        format,
        text,
        sections,
    }
}

/// A line count as the `u32` that chunks store. A page under the 16 MiB
/// fetch limit can't have more than `u32::MAX` lines.
fn line_number(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
```

> **Learn: keeping parsing separate from I/O.** `split_spec` takes `&str` and returns plain data: no file, no network, no cache. That makes every test a one-liner over a string (`include_str!` embeds a fixture at compile time), and the same function will serve the indexer (#25), which already has the bytes from the fetcher. Only the three `#[ignore]` tests touch the network, through `fetch_real`, and they go through the pinned cache, so they check the very bytes the corpus pins.

Run: `cargo test -p codecite-index --lib spec::`
Expected: PASS (11 tests, 3 ignored).

- [ ] **Step 6: Run against the real documents**

Run: `cargo test -p codecite-index --lib spec:: -- --ignored --nocapture`
Expected: PASS, printing `RFC 9449: 48 sections`, `RFC 6749: 116 sections` and `Core: 182 sections`. Keep the output for the PR body.

- [ ] **Step 7: Gates and commit**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`
Expected: all green.

```bash
git add Cargo.toml Cargo.lock deny.toml crates/codecite-index/Cargo.toml crates/codecite-index/src/lib.rs crates/codecite-index/src/spec.rs crates/codecite-index/src/spec/render.rs crates/codecite-index/tests/fixtures/spec/rfc-v3.html crates/codecite-index/tests/fixtures/spec/rfc-text.html crates/codecite-index/tests/fixtures/spec/openid.html
git commit -m "feat(index): split RFC and OpenID spec HTML into numbered sections (#23)"
```

---

### Task 2: Record the parsing approach in design.md

**Files:**
- Modify: `docs/design.md` (Indexing and retrieval, step 3)

- [ ] **Step 1: Edit**

```diff
diff --git a/docs/design.md b/docs/design.md
index 0307df2..8a50ea5 100644
--- a/docs/design.md
+++ b/docs/design.md
@@ -180,7 +180,7 @@ BM25 over code-aware terms does most of the work; vectors and graph signals refi
 
 1. **Fetch:** `git clone --filter=blob:none` at the manifest's pinned commit, read a local working tree, or fetch a document by URL and check it against its pinned `sha256`. Every document is fetched before anything is written, so changed bytes stop the run with an error naming the document and both hashes, and the store is untouched until the pin is updated on purpose. Fetched documents are cached outside the repo, in `<cache dir>/codecite/documents/<sha256>` (`~/.cache` on Linux): a document whose pinned hash is in the cache is never fetched again, so indexing works offline after the first run. A cache file whose bytes no longer match its name is deleted and fetched again. A download is plain HTTP(S) GET (`ureq`, rustls with Mozilla's roots, the standard proxy variables), capped at 16 MiB and 60 seconds; the hash, not the transport or an ETag, is what pins the bytes. A document marked `excluded` is never fetched.
 2. **Walk:** list the files git sees (`git ls-files`, so `.gitignore`, `.git/info/exclude` and the global excludes apply and tracked files are never ignored), apply the manifest's `include`/`exclude` globs, and skip deny-listed secrets, symlinks, files over 1 MiB and binaries (a NUL byte in the first 8000 bytes, git's rule; extensions are never trusted). Every skipped path is reported with its reason; an ignored directory counts once. A path under a symlinked directory is skipped as a symlink. The repository's config is not trusted, since it can name programs for git to run: the walk turns `core.fsmonitor` off, allows git no transport (an empty `GIT_ALLOW_PROTOCOL`, so a partial clone never fetches a missing object from its promisor remote), and never runs `git status`; `dirty` compares HEAD's tree, the index and `git hash-object --no-filters` of the kept files only.
-3. **Chunk:** the language comes from the file extension, then a `python`, `node` or `ts-node` shebang. tree-sitter splits Rust, Python, TypeScript, JavaScript, Go and C# at definitions, one query per language: functions and methods become `function` chunks; classes, structs, enums, traits, interfaces and Rust `impl` blocks become `class` chunks, nested ones included, and each definition but an `impl` block yields a symbol. A definition nested more than 32 deep stays inside its parent's chunk. A symbol's descriptor repeats its parents', so one longer than 512 bytes gives no symbol, to its definition or anything inside it, and a file holds at most 8 MiB of descriptors; past that, definitions keep their chunks without symbols. Together these bound a file's chunking memory by its size. Markdown splits into ATX heading sections, ignoring headings in fenced code. Every other line, and every file in another language, falls into windows of at most 60 lines that never start or end on a line without a letter or digit. Spec documents split into numbered sections and the numbered steps inside them, each with its anchor and its uppercase requirement keywords (BCP 14: RFC 2119 and RFC 8174).
+3. **Chunk:** the language comes from the file extension, then a `python`, `node` or `ts-node` shebang. tree-sitter splits Rust, Python, TypeScript, JavaScript, Go and C# at definitions, one query per language: functions and methods become `function` chunks; classes, structs, enums, traits, interfaces and Rust `impl` blocks become `class` chunks, nested ones included, and each definition but an `impl` block yields a symbol. A definition nested more than 32 deep stays inside its parent's chunk. A symbol's descriptor repeats its parents', so one longer than 512 bytes gives no symbol, to its definition or anything inside it, and a file holds at most 8 MiB of descriptors; past that, definitions keep their chunks without symbols. Together these bound a file's chunking memory by its size. Markdown splits into ATX heading sections, ignoring headings in fenced code. Every other line, and every file in another language, falls into windows of at most 60 lines that never start or end on a line without a letter or digit. Spec documents split into numbered sections and the numbered steps inside them, each with its anchor and its uppercase requirement keywords (BCP 14: RFC 2119 and RFC 8174). Spec HTML comes in three layouts: xml2rfc v3 RFCs (RFC 8650 and later), the older RFC HTML on rfc-editor.org, and the xml2rfc-generated OpenID pages. `scraper` parses it (html5ever repairs malformed HTML as a browser does), and one iterative walk renders it to plain text: a line per paragraph, list item or table row, `<pre>` kept as written, and page headers, footers and ¶ links dropped. A section runs from its heading to the next heading, numbered or not, so its line range holds its own text and not its subsections'. Front matter, the table of contents, the references and unnumbered parts such as the authors' addresses are not sections. html5ever's parse is quadratic in nesting depth, which is fine for the pinned pages the owner chose but slow for a hostile one.
 4. **Terms:** split camelCase, snake_case and kebab-case identifiers into lowercase parts and keep the original, so `validateToken` matches "validate token" and the exact name. Any character that is not a letter or digit separates parts, an acronym ends before the capital that starts the next word (`HTTPServer` → `http server`), and digits stay with the part before them (`sha256`, `rfc9449`).
 5. **Write:** chunks, full-text rows and symbols for one document in a single transaction. A document whose sha256 matches the stored one is not rewritten; only its `dirty` flag is updated. A stored document whose file is gone, or is now skipped, is deleted with everything stored for it, and so is every source the run did not index (one removed from the manifest, or marked `excluded`). A chunk's `terms` and its full-text `terms` column hold the identifier terms of every word in its text; the `symbol` column holds its symbol name's terms and the `path` column its path's. A local source without a `name` is named after its checkout directory, and two sources with one name are an error. The run ends with a summary: documents written, unchanged and removed, chunks written, skipped files by reason, and the elapsed time.
 6. **Providers:** SCIP, CodeGraph and Graphify run after the base pass and add symbols and edges.
```

- [ ] **Step 2: Commit**

```bash
git add docs/design.md
git commit -m "docs: describe how spec HTML is split into sections (#23)"
```
