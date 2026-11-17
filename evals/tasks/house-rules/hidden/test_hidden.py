"""The feature's behaviour: role names are slugs, decided in the domain, and
POST /api/roles/import copies the partner catalogue's roles into the caller's
tenant, all or nothing."""

from __future__ import annotations

import uuid

import pytest
from conftest import CATALOGUE, OTHER_TENANT, SEEN, TENANT


def roles_in(engine) -> dict[tuple[str | None, str], str]:
    """(tenant id or None, name) -> description of every stored role."""
    from sqlmodel import Session, select

    from app.models.identity.role import Role

    with Session(engine) as session:
        return {
            (None if r.tenant_id is None else str(r.tenant_id), r.name): r.description
            for r in session.exec(select(Role)).all()
        }


def seed(engine, tenant: str | None, name: str, description: str = "") -> None:
    from sqlmodel import Session

    from app.models.identity.role import Role

    with Session(engine) as session:
        session.add(
            Role(
                id=uuid.uuid4(),
                name=name,
                description=description,
                tenant_id=None if tenant is None else uuid.UUID(tenant),
            )
        )
        session.commit()


# --- the rule lives in the domain: no database, no service, no router ---


@pytest.mark.parametrize(
    "name",
    ["Billing Admin", "billing admin", "billing_admin", "BillingAdmin", "-billing", "billing-", "billing--admin", "9lives", ""],
)
def test_the_domain_refuses_a_role_name_that_is_not_a_slug(name):
    from app.domain import access

    result = access.role_name(name)
    assert result.is_error(), f"access.role_name({name!r}) should be refused"
    assert isinstance(result.error, access.AccessError)


@pytest.mark.parametrize("name", ["admin", "billing-admin", "tier2-support", "a1-b2-c3"])
def test_the_domain_accepts_a_slug(name):
    from app.domain import access

    assert access.role_name(name).is_ok()


# --- creating and renaming roles ---


def test_creating_a_role_refuses_a_name_that_is_not_a_slug(api, db):
    status, _, _ = api("POST", "/api/roles", json={"name": "Billing Admin"})
    assert status == 422
    assert roles_in(db) == {}
    status, _, _ = api("POST", "/api/roles", json={"name": "billing-admin"})
    assert status == 201
    assert (None, "billing-admin") in roles_in(db)


def test_renaming_a_role_refuses_a_name_that_is_not_a_slug(api, db):
    seed(db, None, "editor", "Can edit")
    status, _, _ = api("PUT", "/api/roles/editor", json={"new_name": "Senior Editor"})
    assert status == 422
    assert roles_in(db) == {(None, "editor"): "Can edit"}


# --- importing the partner catalogue ---


def test_import_creates_the_catalogue_roles_in_the_callers_tenant(api, db):
    seed(db, TENANT, "auditor", "Ours")
    seed(db, OTHER_TENANT, "billing-admin", "Theirs")
    seed(db, None, "support-agent", "Global")
    CATALOGUE[TENANT] = (
        200,
        {
            "roles": [
                {"name": "billing-admin", "description": "Pays invoices"},
                {"name": "auditor", "description": "Reads everything"},
                {"name": "support-agent", "description": "Answers tickets"},
            ]
        },
    )
    status, body, _ = api("POST", "/api/roles/import")
    assert status == 200, body
    assert body["created"] == ["billing-admin", "support-agent"]
    assert body["skipped"] == ["auditor"]
    assert ("GET", f"/tenants/{TENANT}/roles") in SEEN
    assert roles_in(db) == {
        (TENANT, "auditor"): "Ours",
        (TENANT, "billing-admin"): "Pays invoices",
        (TENANT, "support-agent"): "Answers tickets",
        (OTHER_TENANT, "billing-admin"): "Theirs",
        (None, "support-agent"): "Global",
    }


def test_import_reads_the_tenant_from_the_token(api, db):
    CATALOGUE[OTHER_TENANT] = (200, {"roles": [{"name": "auditor", "description": "Reads"}]})
    status, body, _ = api("POST", "/api/roles/import", roles=("owner",), tenant=OTHER_TENANT)
    assert status == 200, body
    assert body["created"] == ["auditor"] and body["skipped"] == []
    assert SEEN == [("GET", f"/tenants/{OTHER_TENANT}/roles")]
    assert roles_in(db) == {(OTHER_TENANT, "auditor"): "Reads"}


def test_import_with_a_name_that_is_not_a_slug_creates_nothing(api, db):
    CATALOGUE[TENANT] = (
        200,
        {"roles": [{"name": "billing-admin", "description": "Pays"}, {"name": "Support Agent", "description": "Helps"}]},
    )
    status, _, _ = api("POST", "/api/roles/import")
    assert status == 422
    assert roles_in(db) == {}


@pytest.mark.parametrize("answer", [(500, {"error": "boom"}), (404, {"error": "no such tenant"}), "drop"])
def test_import_answers_502_when_the_catalogue_fails(api, db, answer):
    CATALOGUE[TENANT] = answer
    status, _, _ = api("POST", "/api/roles/import")
    assert status == 502
    assert roles_in(db) == {}


def test_import_gives_up_on_a_slow_catalogue(api, db):
    CATALOGUE[TENANT] = "hang"
    status, _, seconds = api("POST", "/api/roles/import")
    assert status == 502
    assert seconds < 9, f"the import waited {seconds:.1f}s for the catalogue; it should give up after 5"
    assert roles_in(db) == {}


@pytest.mark.parametrize("roles", [("viewer",), ("member",), ()])
def test_import_is_for_owners_and_admins(api, db, roles):
    CATALOGUE[TENANT] = (200, {"roles": [{"name": "auditor", "description": ""}]})
    status, _, _ = api("POST", "/api/roles/import", roles=roles)
    assert status == 403
    assert roles_in(db) == {}
    assert SEEN == []


def test_import_needs_a_token(api, db):
    CATALOGUE[TENANT] = (200, {"roles": [{"name": "auditor", "description": ""}]})
    status, _, _ = api("POST", "/api/roles/import", auth=False)
    assert status == 401
    assert SEEN == []
