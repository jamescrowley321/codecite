"""Unit tests for the access keys router."""

import time
from unittest.mock import AsyncMock, patch

import pytest
from httpx import ASGITransport, AsyncClient

from app.main import app


# The API requires a bounded, future expiry, so every create needs one. Computed
# per call rather than hardcoded: a fixed timestamp silently rots into the past.
def future_expiry(days: int = 30) -> int:
    return int(time.time()) + days * 86400


@pytest.fixture
def anyio_backend():
    return "asyncio"


@pytest.fixture(autouse=True)
def _reset_rate_limiter():
    """This route is rate limited; without a reset the later tests 429."""
    from app.middleware.rate_limit import limiter

    limiter.reset()


@pytest.fixture(autouse=True)
def _set_env(monkeypatch):
    monkeypatch.setenv("DESCOPE_PROJECT_ID", "test-project-id")
    monkeypatch.setenv("DESCOPE_MANAGEMENT_KEY", "test-management-key")


@pytest.fixture
async def client():
    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as c:
        yield c


ADMIN_CLAIMS = {
    "sub": "user123",
    "dct": "tenant-abc",
    "tenants": {
        "tenant-abc": {"roles": ["admin"], "permissions": ["settings.manage"]},
    },
}

OWNER_CLAIMS = {
    "sub": "owner123",
    "dct": "tenant-abc",
    "tenants": {
        "tenant-abc": {"roles": ["owner"], "permissions": ["settings.manage"]},
    },
}

VIEWER_CLAIMS = {
    "sub": "user456",
    "dct": "tenant-abc",
    "tenants": {
        "tenant-abc": {"roles": ["viewer"], "permissions": ["projects.read"]},
    },
}

KEY_IN_TENANT = {"id": "key123", "name": "Test", "status": "active", "keyTenants": [{"tenantId": "tenant-abc"}]}
KEY_OTHER_TENANT = {"id": "key999", "name": "Other", "status": "active", "keyTenants": [{"tenantId": "tenant-other"}]}


@pytest.mark.anyio
async def test_list_keys_rejects_unauthenticated(client):
    response = await client.get("/api/keys")
    assert response.status_code == 401


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_list_keys_rejected_for_viewer(mock_validate, client):
    mock_validate.return_value = VIEWER_CLAIMS
    response = await client.get("/api/keys", headers={"Authorization": "Bearer valid.token"})
    assert response.status_code == 403


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_list_keys_as_admin(mock_validate, client):
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.search_access_keys.return_value = [KEY_IN_TENANT]
    app.state.descope_client = mock_client

    response = await client.get("/api/keys", headers={"Authorization": "Bearer valid.token"})
    assert response.status_code == 200
    assert len(response.json()["keys"]) == 1
    mock_client.search_access_keys.assert_called_once_with("tenant-abc")


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_create_key(mock_validate, client):
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.create_access_key.return_value = {
        "key": {"id": "new-key-id", "name": "My API Key"},
        "cleartext": "secret-key-value-shown-once",
    }
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "My API Key", "expire_time": future_expiry()},
    )
    assert response.status_code == 200
    data = response.json()
    assert data["cleartext"] == "secret-key-value-shown-once"


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_create_key_with_options(mock_validate, client):
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.create_access_key.return_value = {"key": {"id": "k1"}, "cleartext": "secret"}
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "Scoped Key", "expire_time": future_expiry(), "role_names": ["admin"]},
    )
    assert response.status_code == 200


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_create_key_rejected_for_viewer(mock_validate, client):
    mock_validate.return_value = VIEWER_CLAIMS
    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "Sneaky Key"},
    )
    assert response.status_code == 403


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_get_key(mock_validate, client):
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.load_access_key.return_value = KEY_IN_TENANT
    app.state.descope_client = mock_client

    response = await client.get("/api/keys/key123", headers={"Authorization": "Bearer valid.token"})
    assert response.status_code == 200
    assert response.json()["name"] == "Test"


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_get_key_cross_tenant_rejected(mock_validate, client):
    """Loading a key from another tenant should be rejected."""
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.load_access_key.return_value = KEY_OTHER_TENANT
    app.state.descope_client = mock_client

    response = await client.get("/api/keys/key999", headers={"Authorization": "Bearer valid.token"})
    assert response.status_code == 403


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_deactivate_key(mock_validate, client):
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.load_access_key.return_value = KEY_IN_TENANT
    app.state.descope_client = mock_client

    response = await client.post("/api/keys/key123/deactivate", headers={"Authorization": "Bearer valid.token"})
    assert response.status_code == 200
    assert response.json()["status"] == "deactivated"
    mock_client.deactivate_access_key.assert_called_once_with("key123")


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_deactivate_key_cross_tenant_rejected(mock_validate, client):
    """Deactivating a key from another tenant should be rejected."""
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.load_access_key.return_value = KEY_OTHER_TENANT
    app.state.descope_client = mock_client

    response = await client.post("/api/keys/key999/deactivate", headers={"Authorization": "Bearer valid.token"})
    assert response.status_code == 403


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_activate_key(mock_validate, client):
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.load_access_key.return_value = KEY_IN_TENANT
    app.state.descope_client = mock_client

    response = await client.post("/api/keys/key123/activate", headers={"Authorization": "Bearer valid.token"})
    assert response.status_code == 200
    assert response.json()["status"] == "activated"


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_delete_key(mock_validate, client):
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.load_access_key.return_value = KEY_IN_TENANT
    app.state.descope_client = mock_client

    response = await client.delete("/api/keys/key123", headers={"Authorization": "Bearer valid.token"})
    assert response.status_code == 200
    assert response.json()["status"] == "deleted"
    mock_client.delete_access_key.assert_called_once_with("key123")


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_create_key_rejected_without_tenant(mock_validate, client):
    mock_validate.return_value = {"sub": "user789", "tenants": {}}
    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "No Tenant Key"},
    )
    assert response.status_code == 403


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_admin_cannot_mint_an_owner_key(mock_validate, client):
    """An admin minting an `owner` key is privilege escalation, not delegation.

    The key's roles land in the `tenants` claim of the session token its cleartext
    exchanges for, so granting `owner` here makes indirectly the grant that
    POST /members/invite refuses to make directly — and the key outlives the
    membership that created it.
    """
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "escalate", "role_names": ["owner"], "expire_time": 0},
    )
    assert response.status_code == 403, f"admin minted an owner key: {response.status_code}"
    mock_client.create_access_key.assert_not_called()


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_owner_may_mint_an_owner_key(mock_validate, client):
    """An owner delegating `owner` is not escalation."""
    mock_validate.return_value = OWNER_CLAIMS
    mock_client = AsyncMock()
    mock_client.create_access_key.return_value = {"key": {"id": "k1"}, "cleartext": "x"}
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "ok", "role_names": ["owner"], "expire_time": future_expiry()},
    )
    assert response.status_code == 200


# --- Role containment (privilege escalation) ---


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_admin_cannot_mint_a_key_carrying_an_unheld_custom_role(mock_validate, client):
    """The old guard special-cased `owner`, so any other role escalated freely."""
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "escalate", "role_names": ["billing-superuser"], "expire_time": future_expiry()},
    )

    assert response.status_code == 403
    assert "billing-superuser" in response.json()["detail"]
    mock_client.create_access_key.assert_not_called()


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_partial_escalation_is_refused_whole(mock_validate, client):
    """One unheld role poisons the request; nothing is minted."""
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "mixed", "role_names": ["admin", "owner"], "expire_time": future_expiry()},
    )

    assert response.status_code == 403
    mock_client.create_access_key.assert_not_called()


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_admin_may_mint_a_key_carrying_its_own_role(mock_validate, client):
    """Scoping a key to roles the caller holds is the normal use of this endpoint."""
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.create_access_key.return_value = {"key": {"id": "k1"}, "cleartext": "x"}
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "scoped", "role_names": ["admin"], "expire_time": future_expiry()},
    )

    assert response.status_code == 200
    mock_client.create_access_key.assert_called_once()


# --- Expiry bounds (persistence) ---


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_omitted_expiry_is_refused(mock_validate, client):
    """Omitting expire_time meant 'never expires', and was the UI's default."""
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "forever"},
    )

    assert response.status_code == 422
    mock_client.create_access_key.assert_not_called()


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_zero_expiry_is_refused(mock_validate, client):
    """Descope treats expireTime 0 as never expiring."""
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "forever", "expire_time": 0},
    )

    assert response.status_code == 422
    mock_client.create_access_key.assert_not_called()


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_past_expiry_is_refused(mock_validate, client):
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "stale", "expire_time": int(time.time()) - 60},
    )

    assert response.status_code == 422
    mock_client.create_access_key.assert_not_called()


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_expiry_beyond_the_cap_is_refused(mock_validate, client):
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "decade", "expire_time": future_expiry(days=366)},
    )

    assert response.status_code == 422
    mock_client.create_access_key.assert_not_called()


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_expiry_at_the_cap_is_accepted(mock_validate, client):
    """The boundary is inclusive, so the UI's longest option still works."""
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.create_access_key.return_value = {"key": {"id": "k1"}, "cleartext": "x"}
    app.state.descope_client = mock_client

    response = await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "max", "expire_time": future_expiry(days=364)},
    )

    assert response.status_code == 200


@pytest.mark.anyio
@patch("app.middleware.auth.validate_token", new_callable=AsyncMock)
async def test_validated_expiry_is_what_reaches_descope(mock_validate, client):
    """The value forwarded must be the caller's, not silently rewritten."""
    mock_validate.return_value = ADMIN_CLAIMS
    mock_client = AsyncMock()
    mock_client.create_access_key.return_value = {"key": {"id": "k1"}, "cleartext": "x"}
    app.state.descope_client = mock_client
    expiry = future_expiry(days=45)

    await client.post(
        "/api/keys",
        headers={"Authorization": "Bearer valid.token"},
        json={"name": "forwarded", "expire_time": expiry},
    )

    assert mock_client.create_access_key.await_args.kwargs["expire_time"] == expiry
