The repository at `/work/project` is jamescrowley321/identity-stack. Make the change this issue
asks for. Stop when you're done.

# security: an admin can mint a never-expiring access key carrying roles they do not hold

Found in a fresh-context review of #425 (auth/authz lens). Not caused by that PR — pre-existing, and worth fixing now that bare-project-id session tokens authenticate (#425 makes access-key tokens a live credential path).

`backend/app/routers/accesskeys.py:47-48` special-cases `owner` only, mirroring `users.py:70-71`. So an `admin` can:

```
POST /api/keys {"role_names": ["<any custom role>"], "expire_time": 0}
```

and receive a credential that never expires, carrying roles the caller does not hold, exchangeable for a session JWT this API accepts. An invite creates a revocable human membership; an access key outlives the membership that created it.

Tenant scoping is sound — `services/descope.py:206` takes `tenant_id` from the `dct` claim, not the request body.

Suggested fix:
- require `set(role_names) ⊆ caller_roles`
- cap `expire_time` (reject 0 / unbounded)

Related, same review: `backend/app/middleware/auth.py:81,120` log authentication rejections at DEBUG, while `GatewayClaimsMiddleware` logs each rejection reason at WARNING. In production a rejected token leaves no evidence of why.
