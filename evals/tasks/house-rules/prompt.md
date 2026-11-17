You are working in identity-stack, checked out at `/work/project`: a FastAPI
backend in `backend/`, a React frontend and Terraform. The change below is to
the backend. Read-only copies of other code and documents the team keeps for
reference, if there are any, are under `/opt/corpus` and `/opt/specs`.

Make this change:

> **Import a tenant's roles from the partner catalogue**
>
> Tenants already define their roles in our partner's portal, and want them
> here without retyping them. Add `POST /api/roles/import`. It imports into
> the caller's current tenant the roles the partner role catalogue lists for
> that tenant.
>
> - The catalogue answers `GET <PARTNER_CATALOGUE_URL>/tenants/<tenant id>/roles`
>   with `{"roles": [{"name": "...", "description": "..."}, ...]}`.
>   `PARTNER_CATALOGUE_URL` is a new setting, the catalogue's base URL,
>   defaulting to `http://partner-catalogue.internal`.
> - Each catalogue role becomes a role of the caller's tenant (not a global
>   role), with the catalogue's description. A role the tenant already has
>   by that name is left as it is.
> - Only the tenant's owners and admins may import.
> - Answer 200 with `{"created": [...], "skipped": [...]}`: the names created
>   and the names the tenant already had, each in catalogue order.
> - If the catalogue can't be reached, answers with an error status, or takes
>   more than 5 seconds, answer 502 and create nothing.
>
> Partner role names are free text ("Billing Admin"); ours shouldn't be. From
> now on a role name, however a role is created or renamed, is a lowercase
> slug: a lowercase letter, then lowercase letters, digits and single hyphens,
> not ending in a hyphen (`billing-admin`, `tier2-support`). A name that isn't
> is refused with 422, by `POST /api/roles` and `PUT /api/roles/{name}` as
> well as by the import, which then creates nothing.

There is no catalogue to call while you work; nothing needs to run against a
real one. The backend's unit tests run with `uv run pytest tests/unit` from
`backend/` (the tests under `tests/unit/repositories` need Postgres, which
isn't available). Don't add dependencies.
