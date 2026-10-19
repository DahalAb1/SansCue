# Environment verification status

Updated October 19, 2026. Results below distinguish teammate-reported live
runs from checks executed in this checkout. A result applies to its recorded
revision and scope; it is not automatically acceptance of later changes.

## October 18 live integration evidence

Sanskriti recorded these results in the pull request descriptions:

| Evidence | Revision / branch | Reported result |
|---|---|---|
| [PR #2](https://github.com/DahalAb1/SansCue/pull/2) | `cfa8fc1`, `feat/speaker-bee-binding` | 88 Rust tests against disposable PostgreSQL; 18 frontend tests; synthetic end-to-end PASS. Live Apple Watch on local Compose: Start listening picked up the recording, transcript reached the room, and question drafts appeared. The report does not establish that those drafts were model-generated. |
| [PR #3](https://github.com/DahalAb1/SansCue/pull/3) | `5af0f1d`, `feat/bedrock-live-transcript` | 92 Rust tests against disposable PostgreSQL; 19 frontend tests. Live Apple Watch transcript visible to speaker and TA on local Compose. A real Bedrock request reached AWS but hit the daily token limit; no live AI draft was observed in that run. |

These reports establish observed local Watch-to-room delivery and operation of
the local Compose stack. They do not record every device-runbook scenario,
production HTTPS, backup/restore, load targets, a successful live Bedrock draft,
or a two-model quality/latency/cost comparison. Retain these passes and verify
the remaining scenarios separately. Room selection and moderator behavior have
changed since these runs, so rehearse the current revision before release.

On October 19, before deployment changes, the frontend production build and all
25 frontend tests passed on `c43c8cd`; all 13 deployment provisioning tests also
passed. These are local code checks, not Docker or hosted acceptance.

## October 19 deployment preparation

At the initial preparation checkpoint on `feat/ec2-deployment`, based on
`c43c8cd`, the production override, Caddy
configuration, private environment templates, preflight validator, restricted
database initializer and Mac SSH-tunnel runbook were added. At that checkpoint no AWS resources were created and no emails
were sent.

- All 26 deployment tests passed, including four actual merged-configuration
  checks using standalone Docker Compose 2.40.3 with synthetic settings. These
  check exposed ports, HTTPS origin, database role wiring, persistent volumes,
  restart policies and the optional container worker; they need no Docker daemon.
- The initializer ran on an isolated native PostgreSQL 17 instance. Each of the
  three application roles had no superuser, create-database, create-role or
  replication privileges, and every service's numbered SQL migrations applied
  successfully under its own role. This does not test image initialization.
- Shell syntax, operator configuration static checks and whitespace checks passed.
- Docker Engine and Caddy are unavailable on this host. Image builds, actual
  production container startup, Caddy validation, certificates, hosted browser
  rehearsal, backup/restore and rollback remain unexecuted release gates.

See the [production runbook](../deploy/PRODUCTION.md) for configuration and
remaining execution steps. Organizer email login is a proposal, not an
implemented access mechanism.

## Open demo and IP deployment update

The user selected immediate room creation without email approval for judging.
`SESSIONS_DEMO_OPEN_ROOMS` enables browser-owned organizer rooms, with audience
permissions unchanged; disabling it withdraws anonymous speaker authority.
Production now includes bounded public API/room/bootstrap limits, request-size
limits, and a Caddy-overwritten client-IP header on a private backend.

On the current deployment branch, 109 Rust workspace tests passed against
fresh native PostgreSQL 17 databases with `--test-threads=1`, and the synthetic
end-to-end acceptance passed. The initial parallel run exposed two existing
Topics generation-reliability tests sharing the same queue; the serial run
passed. All 25 frontend tests and the production build passed. All 29 deploy
utility/configuration tests passed, including actual Compose merge checks.
Sessions Clippy passed. Caddy 2.11.7 adapted both DNS and bare-public-IP configs
with the public short-lived ACME profile; no certificate issuance is implied
by adaptation alone.

A separate t3.small EC2 instance with a 30 GiB encrypted gp3 disk was created
in us-east-1 for SansCue. The existing OpenClaw instance was not modified.
Image builds and hosted acceptance are in progress. Availability requested:
November 9–20, 2026 (America/Chicago), with shutdown permitted outside judging.
Optional email-login work was set aside locally and is not part of this demo.

## Historical October 10 acceptance gates (`411fba0`)

- **Hardware-independent synthetic flow:** passed against the source tree now
  integrated in `dev` as `411fba0`, using three fresh, separate native
  PostgreSQL 17.11 databases. The procedure in
  [`services/acceptance`](../services/acceptance/README.md) replayed the
  synthetic fixture twice and verified binding/outbox delivery, event and
  candidate deduplication, staff-only candidate/evidence, publication, an
  audience rating and written Q&A, staff counts, and audience privacy.
- **Service PostgreSQL suites on the same source tree:** Sessions access 9/9,
  Bee persistence 3/3, Topics persistence 2/2, and Topics generation-reliability
  tests 4/4 passed. The latter cover retry/backoff, permanent failure, timeout,
  atomic completion, slow-generation ingress, lease expiry, and fencing.
- **Database-independent checks on the same source tree:** Sessions unit 6/6,
  Bee unit/contract/pipeline/worker 23/23, Topics unit/contract 3/3; 18 frontend
  tests, frontend typecheck and production build; workspace Clippy and Rust
  formatting passed. The checks used native services/databases and did not use
  Docker.
- **Live Bee/device/API, model quality, AWS, Docker/Compose, production
  performance, and backup/restore:** unverified separate gates. Synthetic input
  is not a live Bee validation, and `stub-v1` is not model-quality evidence.
  The [Bee device runbook](bee-device-verification.md) and
  [model comparison worksheet](model-evaluation-worksheet.md) are prepared
  procedures only; neither has been executed.

## Earlier foundation-only verification record

These results were recorded on an earlier `dev` snapshot, before the current
Bee/Topics/question workflow; they are retained as historical context only:

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

## Remaining Docker and deployment checks

Local Compose startup and live routing were reported in PRs #2 and #3. The
following detailed recovery/isolation checks have no complete recorded pass.
Run them on a Docker/Compose-enabled environment after providing the local
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

Docker Engine is unavailable on the current deployment-preparation host.
Native PostgreSQL migrations, readiness, pool behavior, persistence fixtures,
and application integration tests are not Docker-blocked and must remain in the
regular workflow.
