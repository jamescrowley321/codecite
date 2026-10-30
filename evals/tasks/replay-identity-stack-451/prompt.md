The repository at `/work/project` is jamescrowley321/identity-stack. Make the change this issue
asks for. Stop when you're done.

# idp_links: _verify_user_in_tenant is fail-open by construction

Found in a fresh-context review of #425 (auth/authz lens). No live bug — all three call sites check the result.

`backend/app/routers/idp_links.py:48-79` returns a bool instead of raising. Call sites `:92`, `:112`, `:136` check it, so tenant isolation holds today. But the check is advisory: a fourth route that calls it and drops the return silently loses IdP-link tenant isolation, with no type error and no test failure.

Fix: raise a domain exception handled centrally, so forgetting to check is impossible rather than merely unlikely.
