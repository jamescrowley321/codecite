The repository at `/work/project` is jamescrowley321/identity-model. Make the change this issue
asks for. Stop when you're done.

# fix(py): discovery validation ignores the caller's algorithms allowlist

## Problem

`build_resolved_config` in `py/src/py_identity_model/core/token_validation_logic.py` (~L297 on main) builds the resolved config with `algorithms=[alg]`, where `alg` is the resolved JWK `alg` or the JWT header `alg`. The caller's `TokenValidationConfig.algorithms` allowlist is discarded.

Every `perform_disco=True` path (sync and aio, including the refreshed-JWKS retry) builds its config through this helper, so a caller-supplied algorithm allowlist is not enforced when discovery is used. A token signed with an algorithm outside the allowlist is accepted as long as the issuer's JWKS publishes a matching key (e.g. RS256 accepted when only `["PS256", "ES256", "EdDSA"]` is allowed, as FAPI 2.0 requires). The OIDF FAPI2 `invalid-alternate-alg` RP test detects this.

Manual (`perform_disco=False`) validation is not affected.

## Fix

In `build_resolved_config`, reject (`TokenValidationException`, `token_part="header"`) when `original_config.algorithms` is set and the resolved `alg` is not in it. Behaviour with no allowlist is unchanged.

## Scope

Python only. Go (`allowedAlgs`) and Rust (`check_algorithm` before decode, `rust/src/jwt/mod.rs:99`) enforce the allowlist and are unaffected.
