The repository at `/work/project` is jamescrowley321/terraform-provider-descope. Make the change this issue
asks for. Stop when you're done.

# fix: flaky integration test — TestProjectSettings fails on destroy

## Description

`TestProjectSettings` intermittently fails with `Error deleting project` during `terraform destroy` cleanup, indicating the Descope API sometimes rejects project deletion requests.

## Evidence

- Failed on main branch CI run (2026-03-14, run ID `23078143180`)
- Error occurs during the destroy phase, not the apply phase

## Error

```
project_settings_test.go:35: terraform destroy -auto-approve -no-color -input=false failed:
Error: Error deleting project
--- FAIL: TestProjectSettings (11.18s)
```

## Suggested fix

- Add retry logic to the project delete API call in the resource handler
- Consider adding a longer timeout or delay before destroy to allow Descope API to settle
- Alternatively, mark the test with `t.Flaky()` if available, or add a retry wrapper at the test level

## Labels
- bug
- flaky-test

## Names the tests use

The tests that check this change call these names, which don't exist yet.
Define them as shown.

- `transientRetryBaseWait` in `internal/infra/retry.go`:

  ```go
  transientRetryBaseWait = 2 * time.Second
  ```

- `isRetryableError` in `internal/infra/retry.go`:

  ```go
  // isRetryableError checks if an error is a transient Descope API error that
  // should be retried. Returns the Descope error and true for rate limit errors
  // and server errors (5xx status codes).
  func isRetryableError(err error) (*descope.Error, bool) {
  ```

- `retryWaitDuration` in `internal/infra/retry.go`:

  ```go
  // retryWaitDuration returns the appropriate wait duration for a retryable error.
  // Rate limit errors use the Retry-After header if available, falling back to
  // defaultRetryWait. Server errors use exponential backoff starting at 2 seconds.
  func retryWaitDuration(de *descope.Error, attempt uint) time.Duration {
  ```
