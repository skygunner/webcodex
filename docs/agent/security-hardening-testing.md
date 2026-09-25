# Security Hardening Test Playbook

This playbook validates the security-hardening batch that makes direct shared-key auth
local-only by default, pins GitHub Actions to immutable commits, constrains workflow token
permissions, and redacts Server secrets from `Config` Debug output.

It is a validation guide, not deployment authority. Run real-network checks only on a
disposable development Server. Never expose production credentials in logs or reports.

## Security invariants

The batch must preserve these boundaries:

1. `WEBCODEX_SHARED_KEY_ENABLED=true` enables direct unknown-Bearer fallback only for a
   local-only Server and loopback HTTP request authority.
2. Non-loopback bind, configured public origin, or a proxied/public HTTP Host rejects
   direct shared-key fallback unless `WEBCODEX_SHARED_KEY_REMOTE_ENABLED=true` is also
   explicitly enabled.
3. Invalid managed `wc_*` credentials never downgrade into shared-key auth.
4. `WEBCODEX_OAUTH2_SHARED_KEY_BRIDGE` remains a separate OAuth flow and does not silently
   enable direct Bearer shared-key fallback.
5. `webcodex server init --public-url ...` is safe by default: it enables the OAuth issuer
   but does not configure direct shared-key auth or the shared-key OAuth bridge.
6. `webcodex server init --allow-remote-shared-key` is the explicit legacy/trusted hosted
   opt-in and writes both the direct remote acknowledgement and, with a public URL, the
   shared-key OAuth bridge.
7. Managed bootstrap/PAT/OAuth/Runner credentials retain their existing surface gates.
8. Formatting or logging `Config` never reveals the bootstrap token or private project
   share-session fence.
9. Every GitHub workflow defaults its `GITHUB_TOKEN` to `contents: read`; narrower or
   stronger job-level permissions remain explicit at the job that needs them.
10. Every external GitHub Action `uses:` reference is an immutable 40-character commit
    SHA. Readable version comments do not substitute for the SHA.

## Small focused checks

Run these first:

```bash
python3 scripts/tests/test_workflow_security.py

cargo test -p webcodex --lib config_debug_redacts_bootstrap_token -- --nocapture
cargo test -p webcodex --lib shared_key_fallback_requires_explicit_remote_opt_in -- --nocapture
cargo test -p webcodex --lib auth_middleware_direct_shared_key_requires_remote_opt_in_for_public_authority -- --nocapture
cargo test -p webcodex --lib shared_key_fallback_gated_by_env_and_prefix -- --nocapture
cargo test -p webcodex --lib auth_middleware_lightweight_empty_and_open_paths -- --nocapture
cargo test -p webcodex --lib runtime_status_preserves_allowlisted_effective_config_across_projections -- --nocapture

cargo test -p webcodex-cli server_init_ -- --nocapture
```

As with the Job playbook, verify the actual testcase count. `running 0 tests` is not
validation evidence.

## Direct shared-key matrix

Use synthetic secrets on a disposable Server/HTTP fixture. Expected outcomes:

| Server / request | Base shared-key flag | Remote opt-in | Unknown non-`wc_*` bearer |
| --- | --- | --- | --- |
| loopback bind + loopback Host + no public URL | on | off | accepted as shared-key |
| loopback bind + public/proxied Host | on | off | rejected |
| loopback bind + configured public URL | on | off | rejected |
| non-loopback bind | on | off | rejected |
| public/non-loopback deployment | on | on | accepted as shared-key |
| any deployment, invalid `wc_*` managed prefix | on | any | rejected, never downgraded |

QUIC Runner authentication follows the same local-only principle using the actual accepted connection's local IP: loopback QUIC may use local quick-start shared-key auth, while a connection accepted on a non-loopback local address must reject a direct shared-key principal unless the remote opt-in is enabled. On a disposable multi-interface host, include this as a real-network smoke; do not weaken TLS or bind production QUIC merely to exercise the case.

Also verify a valid managed PAT and valid Runner credential still follow their existing
surface restrictions regardless of shared-key flags.

## `server init` configuration matrix

Inspect the generated protected env file, without printing its bootstrap token:

- default loopback init: `WEBCODEX_SHARED_KEY_ENABLED=true`, no remote opt-in;
- `--listen 0.0.0.0:...`: no shared-key env by default;
- `--public-url https://...`: OAuth issuer enabled, no direct shared-key and no shared-key
  OAuth bridge by default;
- `--public-url https://... --allow-remote-shared-key`: writes
  `WEBCODEX_SHARED_KEY_ENABLED=true`, `WEBCODEX_SHARED_KEY_REMOTE_ENABLED=true`, and
  `WEBCODEX_OAUTH2_SHARED_KEY_BRIDGE=true`.

On a disposable running Server, verify `runtime_status.effective_config.auth` distinguishes
`shared_key_configured`, effective `shared_key_enabled`, and `shared_key_remote_enabled`.
When remote exposure is explicitly enabled, startup logs should contain the high-risk
warning but never the shared key or bootstrap token.

## Config Debug redaction

Search normal diagnostics/logging code for accidental direct `Config` Debug formatting and
exercise the focused test. A failure report may show the word `[REDACTED]` but must never
paste the actual test/operator credential. If new secret-bearing fields are added to
`Config` later, extend the custom Debug implementation rather than deriving Debug again.

## GitHub Actions supply-chain checks

`scripts/tests/test_workflow_security.py` is the repository guard. In addition, a test
agent with network access should resolve each pinned SHA against the intended upstream
action repository/tag and report any mismatch. Do not automatically move pins to a newer
tag during regression testing.

Review job-level `permissions:` after the top-level `contents: read` default. Publication
jobs may legitimately request `packages: write`, `actions: read`, or another narrow scope;
that exception must remain local to the job requiring it.

## Broader regression delegated to test agents

After focused checks pass, test agents should run the existing security/auth matrix and the
normal CI/release workflow validation appropriate to their host. Recommended coverage:

```bash
cargo test -p webcodex --lib auth -- --nocapture
cargo test -p webcodex --lib oauth -- --nocapture
cargo test -p webcodex --lib scope -- --nocapture
cargo test -p webcodex --lib metadata -- --nocapture
cargo test -p webcodex-cli
```

Then run the repository's normal monolithic Server suite and, where available, a disposable
hosted-connect/OAuth smoke. Large/full suites remain external-agent work rather than a
requirement for the implementation session.

## Report format

Record exact branch/SHA, architecture, Rust/Python versions, commands, actual testcase
counts, and classify failures as `PRODUCT REGRESSION`, `TEST BUG`, `ENVIRONMENT FAILURE`,
`FLAKY`, or `NOT RUN`. For auth failures include only credential *kind*, bind/Host/public
origin class, flags, HTTP status, and reason code. Never include plaintext credentials,
Authorization headers, generated env-file contents containing tokens, or OAuth secrets.
