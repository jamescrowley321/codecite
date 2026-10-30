#!/usr/bin/env python3
"""Rework on real PRs, read from their blind peer review comments (#179).

Observational only, never a gate. See README.md for what is counted and how.
Stdlib only, so it runs with a bare `python3`.
"""

import json
import re
from typing import NamedTuple

# The overall result of a review that names no lens.
ALL = "All lenses"

# Canonical lens name -> the other spellings review comments use for it.
LENSES = {
    "Cold Read": ["cold_read"],
    "Edge Cases": ["edge_case", "edge case", "edge_cases"],
    "Acceptance Criteria": ["acceptance"],
    "Security Review": ["security"],
    "Red Team": ["red_team"],
    "Policy & Provenance": ["policy", "policy and provenance"],
    "OWASP Web Top 10": ["owasp_web", "owasp web"],
    "OWASP LLM Top 10": ["owasp_llm", "owasp llm"],
}
_ALIASES = {
    alias: name
    for name, aliases in LENSES.items()
    for alias in [name.lower(), *aliases]
}


class LensResult(NamedTuple):
    """One lens's result in one review. A count the comment doesn't give is None."""

    lens: str
    verdict: str  # "PASS" or "BLOCK"
    must: int | None
    should: int | None
    nit: int | None
    total: int | None  # findings of every severity, when the comment says


_HEADING = re.compile(r"\s*#{1,3}\s*blind peer review\b(?P<rest>.*)", re.IGNORECASE)
_VERDICT = re.compile(r"verdict\W*(PASS|BLOCK)\b", re.IGNORECASE)
_PASS_OR_BLOCK = re.compile(r"\b(PASS|BLOCK)\b", re.IGNORECASE)
_TOTAL = re.compile(r"(\d+) findings?(?:\(s\))?", re.IGNORECASE)
_MUST = re.compile(r"(\d+) MUST FIX", re.IGNORECASE)
_LISTING = re.compile(
    r"^\s*(?P<lens>[A-Z][\w &]+?)\s{2,}(?P<verdict>PASS|BLOCK)\s+"
    r"(?P<total>\d+) finding\(s\), (?P<must>\d+) MUST FIX",
    re.MULTILINE,
)
_JSON_BLOCK = re.compile(r"```json\s*(\{.*?\})\s*```", re.DOTALL)


def lens_name(text):
    """The canonical name of the lens `text` spells, or None."""
    key = re.sub(r"[*`]", "", text).strip().lower()
    return _ALIASES.get(key)


def is_review(body):
    """True when the comment's first line is a blind peer review heading."""
    return _HEADING.match(body.lstrip().split("\n", 1)[0]) is not None


def parse_review(body):
    """The lens results in one review comment: a list of LensResult."""
    first, _, rest = body.lstrip().partition("\n")
    heading = _HEADING.match(first)
    if heading is None:
        return []
    lens = _heading_lens(heading["rest"])
    if lens is not None:
        result = _per_lens(lens, rest)
        return [result] if result else []
    results = _table(rest) or _listing(rest)
    if results:
        return results
    verdict = _VERDICT.search(body)
    if verdict:
        return [LensResult(ALL, verdict[1].upper(), None, None, None, None)]
    return []


def _heading_lens(rest):
    # "— Cold Read", ": Cold Read (#860, round 3)", "— acceptance (codex, …)"
    m = re.match(r"\s*[—:-]\s*([^(]+)", rest)
    return lens_name(m[1]) if m else None


def _per_lens(lens, text):
    block = _JSON_BLOCK.search(text)
    if block:
        try:
            findings = json.loads(block[1]).get("findings")
        except json.JSONDecodeError:
            findings = None
        if isinstance(findings, list):
            severities = [str(f.get("severity", "")).upper() for f in findings]
            must = sum("MUST" in s for s in severities)
            should = sum("SHOULD" in s for s in severities)
            nit = sum("NIT" in s for s in severities)
            verdict = "BLOCK" if must else "PASS"
            return LensResult(lens, verdict, must, should, nit, len(findings))
    for line in text.splitlines():
        verdict = _VERDICT.search(line)
        if verdict:
            total = _TOTAL.search(line)
            must = _MUST.search(line)
            return LensResult(
                lens,
                verdict[1].upper(),
                int(must[1]) if must else None,
                None,
                None,
                int(total[1]) if total else None,
            )
    # No verdict line: "No findings. **PASS**"
    bare = re.findall(r"\*\*(PASS|BLOCK)\*\*", text)
    if bare:
        total = 0 if re.search(r"\bno findings\b", text, re.IGNORECASE) else None
        return LensResult(lens, bare[-1], None, None, None, total)
    return None


def _cells(line):
    return [c.strip() for c in line.strip().strip("|").split("|")]


def _count(cell):
    m = re.search(r"\d+", cell)
    return int(m[0]) if m else None


def _table(text):
    """Rows of a Markdown table whose header has a Lens column."""
    columns = None
    results = []
    for line in text.splitlines():
        if not line.lstrip().startswith("|"):
            if results:
                break
            columns = None
            continue
        cells = _cells(line)
        if columns is None:
            header = [c.lower() for c in cells]
            if header and header[0] == "lens":
                columns = header
            continue
        lens = lens_name(cells[0])
        if lens is None:
            continue
        row = dict(zip(columns, cells))
        result = next(
            (row[c] for c in columns if c in ("result", "verdict", "status")), ""
        )
        verdict = _PASS_OR_BLOCK.search(result)
        if verdict is None:
            continue

        def severity(prefix):
            cell = next((row[c] for c in columns if c.startswith(prefix)), None)
            return None if cell is None else _count(cell)

        total = _TOTAL.search(result)
        results.append(
            LensResult(
                lens,
                verdict[1].upper(),
                severity("must"),
                severity("should"),
                severity("nit"),
                int(total[1]) if total else None,
            )
        )
    return results


def _listing(text):
    """Lines like `  Cold Read   PASS   1 finding(s), 0 MUST FIX`."""
    results = []
    for m in _LISTING.finditer(text):
        lens = lens_name(m["lens"])
        if lens is not None:
            results.append(
                LensResult(
                    lens, m["verdict"], int(m["must"]), None, None, int(m["total"])
                )
            )
    return results


SPEC = "Acceptance Criteria"
CONVENTION = "Policy & Provenance"


def rounds(reviews):
    """Group `(createdAt, [LensResult])` reviews into rounds, oldest first.

    A combined comment is a round of its own. Per-lens comments in a row form
    one round, which ends when a lens repeats.
    """
    out = []
    run = None  # the open per-lens round, if any
    for _, results in sorted(reviews, key=lambda r: r[0]):
        if not results:
            continue
        per_lens = len(results) == 1 and results[0].lens != ALL
        if not per_lens:
            out.append(list(results))
            run = None
        elif run is not None and results[0].lens not in {r.lens for r in run}:
            run.append(results[0])
        else:
            run = [results[0]]
            out.append(run)
    return out


def rounds_to_pass(rounds):
    """The 1-based index of the first round where every lens passed, or None."""
    for i, results in enumerate(rounds, 1):
        if all(r.verdict == "PASS" for r in results):
            return i
    return None


def _findings(result):
    """MUST + SHOULD findings, or the lens's total when neither is given."""
    if result.must is None and result.should is None:
        return result.total or 0
    return (result.must or 0) + (result.should or 0)


def violations(rounds):
    """Spec, convention and all MUST + SHOULD findings across every round."""
    counts = {"spec": 0, "convention": 0, "all": 0}
    for results in rounds:
        for r in results:
            n = _findings(r)
            counts["all"] += n
            if r.lens == SPEC:
                counts["spec"] += n
            elif r.lens == CONVENTION:
                counts["convention"] += n
    return counts
