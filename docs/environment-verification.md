# Environment verification status

Updated October 9, 2026. The earlier native-PostgreSQL/frontend results below
are a historical foundation record, not a claim that the full current MVP or
the new synthetic acceptance procedure has run on this checkout. This document
tracks Docker-only checks and does not replace current service-specific
PostgreSQL verification.

## Current acceptance gates

- **Hardware-independent synthetic flow:** the procedure at
  [`services/acceptance`](../services/acceptance/README.md) is available and
  requires three distinct native PostgreSQL databases. Its result is pending
  until explicitly run on the target commit.
- **Local code checks:** current branch checks should be reported with the exact
  commit and the observed frontend/Rust commands; prior foundation results below
  should not be generalized to newly added services.
- **Live Bee/device/API, model quality, AWS, Docker/Compose, production
  performance, and backup/restore:** unverified separate gates. Synthetic input
  is not a live Bee validation, and `stub-v1` is not model-quality evidence.

## Passed without containers

On `dev` after the live-updates, local-runtime, and room-reconnect squash
integrations:

- Rust formatting and Clippy (`cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`).
- `cargo test --workspace` against the provisioned native PostgreSQL 17 database:
  6 library unit tests, 5 access/integration tests, and 4 readiness/migration
  tests passed. These exercise database migrations, repeat startup, readiness,
  pool timeout/recovery, migration failure, and startup bounds.
- Frontend typecheck, all 14 frontend tests (including live-event replay
  scoping and reconnect backoff), and production build.
- 13 operator-provisioning unit tests, static Compose operator-configuration
  checks, and `git diff --check`.

## Pending: Docker-only checks

Run these on a Docker/Compose-enabled environment after providing the local
`deploy/.env` configuration described in [the runtime guide](../deploy/README.md).
Do not print or publish resolved Compose configuration, container environments,
or logs containing credentials.

1. **Build-context isolation:** `python3 deploy/check-build-context.py`. This uses
   Docker/BuildKit for two isolated scratch builds and checks the 17 allowed
   inputs against harmless credential/generated fixtures and source-shaped
   descendant paths.
2. **Compose configuration and images:**
   `docker compose --env-file deploy/.env config --quiet`, then
   `docker compose --env-file deploy/.env up --build --wait --wait-timeout 120`.
   Confirm all services are healthy with `docker compose --env-file deploy/.env
   ps` (inspect logs locally only if needed).
3. **HTTP routing and health through Caddy:** check `/`,
   `/api/sessions/healthz`, `/api/sessions/readyz`, unknown sessions/API paths,
   and a client route on `http://127.0.0.1:8080` as listed in the runtime guide.
   Confirm unknown API paths are 404, health/readiness are 200, and unavailable
   upstreams do not return SPA HTML.
4. **Compose database outage/recovery:** with the stack running, stop only
   `sessions-db`, confirm proxied liveness stays 200 and readiness becomes 503,
   start `sessions-db` again, and confirm readiness returns to 200 without
   restarting sessions. Use the exact stop/start sequence in the runtime guide.
5. **Real post-bind outage/recovery:** run
   `python3 services/sessions/tests/outage-recovery.py`. It must kill and restart
   its own disposable PostgreSQL 17 container, observe liveness 200/readiness
   503 during outage and readiness recovery within 30 seconds, with the same
   sessions process alive throughout.
6. **Named-volume persistence:** record migration metadata (or a disposable
   local test marker), run ordinary `docker compose --env-file deploy/.env down`
   and `up -d --wait --wait-timeout 120`, then confirm the data remains. Do not
   use `down -v` for this check.
7. **Backup/restore execution:** create a private custom-format backup with
   `pg_dump`, validate it with `pg_restore --list`, restore it following the
   documented writer-stop procedure, then verify readiness and compare the
   restored data. Keep the backup private and remove any disposable test marker.

These are environment-blocked only because Docker/Compose is unavailable here.
Native PostgreSQL migrations, readiness, pool behavior, persistence fixtures,
and application integration tests are not Docker-blocked and must remain in the
regular workflow.
