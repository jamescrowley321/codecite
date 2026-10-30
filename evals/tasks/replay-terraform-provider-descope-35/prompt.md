The repository at `/work/project` is jamescrowley321/terraform-provider-descope. Make the change this issue
asks for. Stop when you're done.

# feat: add descope_access_key resource

## Summary

Add a `descope_access_key` resource for managing Descope access keys (M2M API keys) via the Management API.

## Background

The upstream repo has an open issue for this: [feat: M2M access keys](https://github.com/descope/terraform-provider-descope/issues). Access keys are critical for machine-to-machine authentication and API access.

## API Coverage (from Go SDK `AccessKey` interface)

- [ ] `Create` — Create access key with name, expiration, roles, tenants, user ID, custom claims
- [ ] `Update` — Update access key name
- [ ] `Delete` — Delete access key
- [ ] `Load` — Load access key by ID
- [ ] `SearchAll` — Search all access keys with filters
- [ ] `Activate` / `Deactivate` — Status management

## Proposed Schema

```hcl
resource "descope_access_key" "api_key" {
  name       = "production-api-key"
  expire_time = 0  # 0 = never expires
  
  roles = ["api-reader"]
  
  tenant {
    tenant_id = descope_tenant.acme.id
    roles     = ["tenant-api-access"]
  }
  
  user_id       = descope_user.service_account.id  # Optional
  custom_claims = {                                  # Optional
    service = "backend"
  }
}

output "cleartext_key" {
  value     = descope_access_key.api_key.cleartext
  sensitive = true
}
```

## Implementation Notes

- The `cleartext` value is only available at creation time — mark as `Sensitive: true` and store in state
- Support `ForceNew` on immutable fields (expiration, user_id)
- Consider a `descope_access_key` data source for loading existing keys (without cleartext)
- Handle key rotation lifecycle (new key before old key is deleted)

## Acceptance Criteria

- [ ] `descope_access_key` resource with full CRUD
- [ ] `descope_access_key` data source for read-only lookups
- [ ] Cleartext properly handled as sensitive
- [ ] Acceptance tests
- [ ] Documentation
