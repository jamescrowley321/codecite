The repository at `/work/project` is jamescrowley321/codecite. Make the change this issue
asks for. Stop when you're done.

# feat(mcp): add a requirements tool that lists every rule governing a spec section

## Why
The harder eval tasks fail because agents never look rules up, not because search ranks badly. In the quick calibrations, every failed backchannel-logout run missed a rule in Back-Channel Logout 1.0 §2.4:
- gpt-6.1-sol never required `exp`.
- Haiku never checked that the logout event's value is a JSON object.

The agents didn't search and miss; they didn't know there was a rule to look for. A better search engine doesn't reach an agent that never searches. What would reach it is one call that lays out every rule that applies.

## What
A `requirements` tool (MCP, plus a CLI command). Given a section citation, or a topic and a document, it returns every normative statement that governs it, each with its pinned citation:
- every step and sentence with MUST, MUST NOT, REQUIRED, SHALL, SHOULD or SHOULD NOT;
- the REQUIRED and PROHIBITED rows of claim and parameter tables (for example, §2.4's `exp: REQUIRED`).

It spans the governing sections: the section asked for, its subsections, and the sections it names by reference within the same document (for example, §2.6's steps plus §2.4's claim table). It groups them by section, MUSTs first.

The index already tags requirement keywords and steps (#24), so this reads what's stored and adds no new indexing.

## Done when
1. `codecite requirements <corpus> <citation>` and the MCP `requirements` tool return the statements above, cited and grouped.
2. **Test, Back-Channel Logout §2.6:** the result includes `exp` REQUIRED (§2.4), the events member's value MUST be a JSON object (§2.4), `nonce` MUST NOT be present, and steps 2–8.
3. **Test, RFC 6749 §2.3:** the result includes "MUST NOT use more than one authentication method" (§2.3), "MUST NOT be included in the request URI" (§2.3.1) and, through the reference to §3.2, "MUST NOT be included more than once".
4. `docs/usage.md` documents the tool, and the server instructions say when to use it ("before implementing or fixing anything a spec governs").
5. A quick-tier A/B run in CI on backchannel-logout and client-auth-uri has its scorecard posted on the PR.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
