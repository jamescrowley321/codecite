"""Unit tests for the require_fga dependency factory and extract_user_id helper."""

import logging
from unittest.mock import AsyncMock, MagicMock

import httpx
import pytest
from fastapi import HTTPException

from app.dependencies.fga import extract_user_id, require_fga, resolve_fga_user_target


def _make_request(claims, path_params=None, descope_client=None):
    request = MagicMock()
    request.state.claims = claims
    request.path_params = path_params or {}
    request.app.state.descope_client = descope_client
    return request


def _make_http_status_error(status_code=500):
    req = httpx.Request("POST", "https://api.descope.com/v1/mgmt/authz/has")
    resp = httpx.Response(status_code, request=req, text="error")
    return httpx.HTTPStatusError(f"{status_code}", request=req, response=resp)


def _make_request_error():
    req = httpx.Request("POST", "https://api.descope.com/v1/mgmt/authz/has")
    return httpx.RequestError("Connection refused", request=req)


VALID_CLAIMS = {"sub": "user-abc", "dct": "tenant-1"}


class TestExtractUserId:
    def test_returns_user_id(self):
        assert extract_user_id(_make_request(VALID_CLAIMS)) == "user-abc"

    def test_no_claims_attribute_raises_401(self):
        request = MagicMock()
        request.state = MagicMock(spec=[])
        with pytest.raises(HTTPException) as exc_info:
            extract_user_id(request)
        assert exc_info.value.status_code == 401
        assert exc_info.value.detail == "Not authenticated"

    def test_non_dict_claims_raises_401(self):
        """Claims set to a non-dict type (e.g., string) -> 401."""
        with pytest.raises(HTTPException) as exc_info:
            extract_user_id(_make_request("not-a-dict"))
        assert exc_info.value.status_code == 401
        assert exc_info.value.detail == "Not authenticated"

    def test_none_claims_raises_401(self):
        with pytest.raises(HTTPException) as exc_info:
            extract_user_id(_make_request(None))
        assert exc_info.value.status_code == 401

    def test_missing_sub_raises_401(self):
        with pytest.raises(HTTPException) as exc_info:
            extract_user_id(_make_request({"dct": "tenant-1"}))
        assert exc_info.value.status_code == 401
        assert exc_info.value.detail == "Missing user identity"

    def test_empty_sub_raises_401(self):
        with pytest.raises(HTTPException) as exc_info:
            extract_user_id(_make_request({"sub": "", "dct": "t1"}))
        assert exc_info.value.status_code == 401


class TestRequireFga:
    @pytest.mark.anyio
    async def test_allowed_returns_user_id(self):
        mock_client = AsyncMock()
        mock_client.check_permission.return_value = True

        dep = require_fga("document", "can_view")
        result = await dep(_make_request(VALID_CLAIMS, {"document_id": "doc-123"}, descope_client=mock_client))
        assert result == "user-abc"
        # resource_id is tenant-prefixed
        mock_client.check_permission.assert_called_once_with("document", "tenant-1:doc-123", "can_view", "user-abc")

    @pytest.mark.anyio
    async def test_denied_raises_403(self):
        mock_client = AsyncMock()
        mock_client.check_permission.return_value = False

        dep = require_fga("document", "can_edit")
        with pytest.raises(HTTPException) as exc_info:
            await dep(_make_request(VALID_CLAIMS, {"document_id": "doc-123"}, descope_client=mock_client))
        assert exc_info.value.status_code == 403
        assert exc_info.value.detail == "Access denied"

    @pytest.mark.anyio
    async def test_denied_logs_warning(self, caplog):
        """AC3: FGA denial is logged with warning level for audit trail."""
        mock_client = AsyncMock()
        mock_client.check_permission.return_value = False

        dep = require_fga("document", "can_edit")
        with caplog.at_level(logging.WARNING, logger="app.dependencies.fga"):
            with pytest.raises(HTTPException):
                await dep(_make_request(VALID_CLAIMS, {"document_id": "doc-123"}, descope_client=mock_client))
        assert any(r.levelno == logging.WARNING and "FGA denied" in r.message for r in caplog.records)
        assert any("user-abc" in r.message for r in caplog.records)

    @pytest.mark.anyio
    async def test_no_claims_attribute_raises_401(self):
        request = MagicMock()
        request.state = MagicMock(spec=[])
        request.path_params = {"document_id": "doc-123"}

        dep = require_fga("document", "can_view")
        with pytest.raises(HTTPException) as exc_info:
            await dep(request)
        assert exc_info.value.status_code == 401
        assert exc_info.value.detail == "Not authenticated"

    @pytest.mark.anyio
    async def test_none_claims_raises_401(self):
        dep = require_fga("document", "can_view")
        with pytest.raises(HTTPException) as exc_info:
            await dep(_make_request(None, {"document_id": "doc-123"}))
        assert exc_info.value.status_code == 401

    @pytest.mark.anyio
    async def test_missing_sub_raises_401(self):
        dep = require_fga("document", "can_view")
        with pytest.raises(HTTPException) as exc_info:
            await dep(_make_request({"dct": "t-1"}, {"document_id": "doc-123"}))
        assert exc_info.value.status_code == 401
        assert exc_info.value.detail == "Missing user identity"

    @pytest.mark.anyio
    async def test_empty_sub_raises_401(self):
        dep = require_fga("document", "can_view")
        with pytest.raises(HTTPException) as exc_info:
            await dep(_make_request({"sub": "", "dct": "t1"}, {"document_id": "doc-123"}))
        assert exc_info.value.status_code == 401

    @pytest.mark.anyio
    async def test_http_error_raises_502_fail_closed(self):
        mock_client = AsyncMock()
        mock_client.check_permission.side_effect = _make_http_status_error(500)

        dep = require_fga("document", "can_view")
        with pytest.raises(HTTPException) as exc_info:
            await dep(_make_request(VALID_CLAIMS, {"document_id": "doc-123"}, descope_client=mock_client))
        assert exc_info.value.status_code == 502
        assert exc_info.value.detail == "Authorization check failed"

    @pytest.mark.anyio
    async def test_network_error_raises_502_fail_closed(self):
        mock_client = AsyncMock()
        mock_client.check_permission.side_effect = _make_request_error()

        dep = require_fga("document", "can_view")
        with pytest.raises(HTTPException) as exc_info:
            await dep(_make_request(VALID_CLAIMS, {"document_id": "doc-123"}, descope_client=mock_client))
        assert exc_info.value.status_code == 502

    @pytest.mark.anyio
    async def test_none_result_treated_as_denied(self):
        """check_permission returns None -> treated as denied (fail-closed)."""
        mock_client = AsyncMock()
        mock_client.check_permission.return_value = None

        dep = require_fga("document", "can_view")
        with pytest.raises(HTTPException) as exc_info:
            await dep(_make_request(VALID_CLAIMS, {"document_id": "doc-123"}, descope_client=mock_client))
        assert exc_info.value.status_code == 403

    @pytest.mark.anyio
    async def test_custom_resource_id_param(self):
        """Verify factory uses the custom resource_id_param name."""
        mock_client = AsyncMock()
        mock_client.check_permission.return_value = True

        dep = require_fga("folder", "can_delete", resource_id_param="folder_id")
        await dep(_make_request(VALID_CLAIMS, {"folder_id": "folder-99"}, descope_client=mock_client))
        # resource_id is tenant-prefixed
        mock_client.check_permission.assert_called_once_with("folder", "tenant-1:folder-99", "can_delete", "user-abc")

    @pytest.mark.anyio
    async def test_missing_resource_id_raises_400(self):
        """Path param not present -> 400."""
        dep = require_fga("document", "can_view")
        with pytest.raises(HTTPException) as exc_info:
            await dep(_make_request(VALID_CLAIMS, {}))
        assert exc_info.value.status_code == 400
        assert exc_info.value.detail == "Missing resource identifier"

    @pytest.mark.anyio
    async def test_missing_tenant_id_raises_403(self):
        """Claims with sub but no dct -> 403 (no tenant context)."""
        dep = require_fga("document", "can_view")
        with pytest.raises(HTTPException) as exc_info:
            await dep(_make_request({"sub": "user-abc"}, {"document_id": "doc-123"}))
        assert exc_info.value.status_code == 403
        assert exc_info.value.detail == "No tenant context"


CALLER_TENANT = "tenant-1"
OTHER_TENANT = "tenant-2"
VICTIM_USER_ID = "U3JvictimInternalId"


def _client_returning(user):
    client = MagicMock()
    client.load_user = AsyncMock(return_value=user)
    return client


def _client_raising(exc):
    client = MagicMock()
    client.load_user = AsyncMock(side_effect=exc)
    return client


class TestResolveFgaUserTargetTenantScoping:
    """The resolver must not turn /api/fga/* into a project-wide existence oracle."""

    @pytest.mark.asyncio
    async def test_resolves_user_inside_the_callers_tenant(self):
        client = _client_returning({"userId": VICTIM_USER_ID, "userTenants": [{"tenantId": CALLER_TENANT}]})
        result = await resolve_fga_user_target(client, "member@example.com", CALLER_TENANT)
        assert result == VICTIM_USER_ID

    @pytest.mark.asyncio
    async def test_user_in_another_tenant_is_left_unresolved(self):
        """A foreign-tenant hit must not leak the internal userId."""
        client = _client_returning({"userId": VICTIM_USER_ID, "userTenants": [{"tenantId": OTHER_TENANT}]})
        result = await resolve_fga_user_target(client, "victim@other-tenant.example", CALLER_TENANT)
        assert result == "victim@other-tenant.example"
        assert VICTIM_USER_ID not in result

    @pytest.mark.asyncio
    async def test_foreign_tenant_is_indistinguishable_from_nonexistent(self):
        """The oracle closes only if both cases return the identical value."""
        identifier = "probe@example.com"

        foreign = await resolve_fga_user_target(
            _client_returning({"userId": VICTIM_USER_ID, "userTenants": [{"tenantId": OTHER_TENANT}]}),
            identifier,
            CALLER_TENANT,
        )
        missing = await resolve_fga_user_target(
            _client_raising(_make_http_status_error(404)), identifier, CALLER_TENANT
        )
        assert foreign == missing == identifier

    @pytest.mark.asyncio
    async def test_multi_tenant_user_resolves_when_one_membership_matches(self):
        client = _client_returning(
            {
                "userId": VICTIM_USER_ID,
                "userTenants": [{"tenantId": OTHER_TENANT}, {"tenantId": CALLER_TENANT}],
            }
        )
        result = await resolve_fga_user_target(client, "shared@example.com", CALLER_TENANT)
        assert result == VICTIM_USER_ID

    @pytest.mark.asyncio
    async def test_user_with_no_tenants_is_left_unresolved(self):
        client = _client_returning({"userId": VICTIM_USER_ID, "userTenants": []})
        result = await resolve_fga_user_target(client, "orphan@example.com", CALLER_TENANT)
        assert result == "orphan@example.com"

    @pytest.mark.asyncio
    async def test_missing_user_tenants_key_is_left_unresolved(self):
        """An absent userTenants must not be read as 'in every tenant'."""
        client = _client_returning({"userId": VICTIM_USER_ID})
        result = await resolve_fga_user_target(client, "nokey@example.com", CALLER_TENANT)
        assert result == "nokey@example.com"

    @pytest.mark.asyncio
    async def test_malformed_tenant_entries_are_ignored(self):
        client = _client_returning(
            {"userId": VICTIM_USER_ID, "userTenants": ["not-a-dict", None, {"tenantId": CALLER_TENANT}]}
        )
        result = await resolve_fga_user_target(client, "odd@example.com", CALLER_TENANT)
        assert result == VICTIM_USER_ID

    @pytest.mark.asyncio
    async def test_non_user_subject_passes_through(self):
        """The admin surface must still accept subjects that are not users at all."""
        client = _client_raising(_make_http_status_error(404))
        result = await resolve_fga_user_target(client, "service-account-7", CALLER_TENANT)
        assert result == "service-account-7"

    @pytest.mark.asyncio
    async def test_upstream_fault_is_not_swallowed(self):
        client = _client_raising(_make_http_status_error(500))
        with pytest.raises(HTTPException) as exc_info:
            await resolve_fga_user_target(client, "member@example.com", CALLER_TENANT)
        assert exc_info.value.status_code == 502

    @pytest.mark.asyncio
    async def test_network_error_is_not_swallowed(self):
        client = _client_raising(_make_request_error())
        with pytest.raises(HTTPException) as exc_info:
            await resolve_fga_user_target(client, "member@example.com", CALLER_TENANT)
        assert exc_info.value.status_code == 502
