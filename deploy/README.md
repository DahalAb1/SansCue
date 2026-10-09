# Local development runtime

Local-only packaging of the static Preact/Vite build behind Caddy, Sessions, Bee-connection, Topics-and-questions, and their separate PostgreSQL databases. Run commands from the repository root. This is not a production configuration.

## Prerequisites and integration

- Docker Engine/Desktop with a running daemon and Compose v2 supporting `up --wait --wait-timeout`; host `curl` for checks.
- Web expects `web/package.json`, `web/package-lock.json`, `npm run build`, and output `web/dist/`. Sessions, Bee-connection, and Topics-and-questions are independent Rust workspace packages; each applies only its own DB migrations.
- Free loopback ports 8080 and 5432; host development also needs 3000 and 5173.
- Initial builds need image/package registry access. Official Node, Caddy, Rust, Debian, and PostgreSQL images provide the runtime; no custom orchestration.

Dockerfiles use repository-root build contexts. Root `.dockerignore` defaults to deny and allows only the current manifests, build configuration, Rust/SQL source, and TypeScript/TSX/CSS source. New source or asset types require an explicit allowlist update; do not reinclude whole application directories. Sensitive directories and credential filenames are also denied after the allowlist. Terminal descendant exclusions for every allowed path prevent extension-shaped or manifest-named directories from admitting arbitrary children; keep these exclusions last when extending the allowlist. Run `python3 deploy/check-build-context.py` with Docker/BuildKit to verify the explicit required-input allowlist and ensure harmless credential/generated fixtures and descendants of source-shaped paths are excluded using isolated scratch builds (no actual credentials or application directories are copied). No database configuration enters the browser/web build. Application images explicitly install `curl` for probes. PostgreSQL 17 includes `pg_isready`, `psql`, `pg_dump`, and `pg_restore`.

## Configure and start

```sh
cp deploy/.env.example deploy/.env
chmod 600 deploy/.env
# Edit deploy/.env locally; never commit it. Replace all local-only service DB passwords and service tokens.
docker compose --env-file deploy/.env config --quiet
docker compose --env-file deploy/.env up --build --wait --wait-timeout 120
docker compose --env-file deploy/.env ps
docker compose --env-file deploy/.env logs --tail=100 web sessions sessions-db bee-connection topics-and-questions
```

Keep database `sessions` and local role `sessions_app`. Bee and Topics each use their own DB, role, password, and volume. Sample passwords/tokens are local-only and not production-ready; provision distinct random service tokens outside Git for any non-local use. Use only URL-unreserved password characters (letters, digits, `. _ ~ -`): Compose interpolates them directly into the private database URLs. Do not export competing `POSTGRES_*` shell values: those override the env file. If needed, inspect `docker compose --env-file deploy/.env config` only locally. **Its resolved output contains credentials; never publish it, container environment inspection, or credentials in logs/issues.**

PostgreSQL creates the database and role on first initialization only. Its `POSTGRES_USER` is a local bootstrap superuser, not a least-privilege production role design. Changing `POSTGRES_*` does not update an existing database/password. Sessions owns and applies migrations and its pool; runtime creates no product tables or separate migration runner.

## Operator provisioning

Operator access is optional for stack startup but **required to create rooms**. Compose passes `SESSIONS_OPERATOR_TOKEN_SHA256`, `SESSIONS_OPERATOR_EPOCH`, and `SESSIONS_IDEMPOTENCY_HMAC_KEY` from `deploy/.env` to sessions at container runtime only (never build args, image layers, or Git). With any of the three blank or absent, sessions starts healthy but operator login and room creation stay disabled (fail closed). There are no defaults. A malformed hash or key (not exactly 64 hex characters) makes sessions exit at startup.

- `SESSIONS_OPERATOR_TOKEN_SHA256`: SHA-256 hex of a random 256-bit operator credential. Only the hash goes in `deploy/.env`.
- `SESSIONS_OPERATOR_EPOCH`: separate, non-secret label (letters, digits, `. _ -`). Changing it ends existing operator sessions.
- `SESSIONS_IDEMPOTENCY_HMAC_KEY`: independent random 256-bit key, 64 hex characters. Never derive it from or reuse the credential or its hash.

Run from the repository root with Python 3 (standard library only). `deploy/provision_operator.py` is a local-only utility. Each run generates an operator credential, its SHA-256, a unique random epoch (UTC time plus 128 random bits, never reused, so rapid rotations cannot collide), and an independent random HMAC key. It then atomically replaces `deploy/.env` (mode 600; created from `.env.example` if absent, other settings kept) with only the hash, epoch, and HMAC key, and atomically replaces `local-data/operator/credential` (mode 600, in mode-700 `local-data/` and `local-data/operator/`) with the plaintext credential. It prints no credential, hash, or key, reads nothing secret from arguments or the environment, and refuses symbolic-link targets. If the second write fails, the credential file is restored so the pair stays consistent; rerun to retry.

```sh
python3 deploy/provision_operator.py
docker compose --env-file deploy/.env up -d --wait --wait-timeout 120 sessions
```

Read the credential privately as the trusted operator, on your own terminal, with no screen sharing, recording, or shared scrollback. Open `local-data/operator/credential` in a local editor or password manager, or run `cat local-data/operator/credential` once; the file holds exactly the credential with no trailing newline. Copy it into the operator login of the local app, or store it in a password manager and delete the file. Never paste it into issues, chat, shell arguments, `docker compose` arguments, or logs, and never add it to `deploy/.env`, Git, or an image.

**Rerunning rotates everything:** `python3 deploy/provision_operator.py` replaces the credential, hash, epoch, and HMAC key together, so the old credential stops working and, because the epoch changes, prior operator sessions end after sessions is recreated. To disable the operator again, blank all three values and recreate `sessions`. `deploy/.env`, its temporary `deploy/.env.*.tmp` files, and `local-data/` are Git-ignored and excluded from Docker build contexts; keep them out of backups you share. Container environment inspection and `docker compose config` expose the hash, epoch, and HMAC key, so keep them private too.

Check the utility without touching your real files (tests use a temporary directory; `-B` avoids writing bytecode):

```sh
python3 -B -m unittest discover -s deploy -p 'test_*.py'
```

## Stack endpoints

- Browser: `http://127.0.0.1:8080`; Caddy listens on container `:80`.
- Sessions: `sessions:3000` public app API and `sessions:3001` private service listener, bound on the container network; neither port is published.
- Topics-and-questions: `topics-and-questions:3002`, private container network only.
- PostgreSQL: three independent private DBs at `sessions-db:5432`, `bee-db:5432`, and `topics-db:5432`; only Sessions DB publishes `127.0.0.1:5432`.

The Bee worker handles room-binding commands and retries accepted canonical transcript events through its transactional outbox. Its deterministic replay harness remains a separate local command; Compose does not start or claim a real Bee device transport. Topics processes one `stub-v1` development preview per accepted event and retries candidate delivery to Sessions. The preview is unpublished and visible only to room speakers/TAs; it is not model-generated and never appears to audience members. Live Bee device/API verification remains separate.

Startup gates database health, then sessions readiness, then Caddy health. Sessions readiness uses its database pool; Caddy health probes its static root independently. Dependency gates apply at startup, not continuous supervision: a later database outage does not stop Caddy or restart sessions automatically. The 120-second wait is a health-wait bound, not an image download/build deadline or the service startup bound (documented by the sessions implementation).

## Verify routing and recovery

```sh
curl -i http://127.0.0.1:8080/
curl -i http://127.0.0.1:8080/api/sessions/healthz
curl -i http://127.0.0.1:8080/api/sessions/readyz
curl -i http://127.0.0.1:8080/api/sessions/not-a-route
curl -i http://127.0.0.1:8080/api/not-a-service
curl -i http://127.0.0.1:8080/api/sessions-extra/healthz
curl -i http://127.0.0.1:8080/a-client-route
```

Health/readiness should return 200 JSON `{"status":"ok"}` and `{"status":"ready"}`. Unknown API paths must return 404, never SPA HTML. Caddy strips exactly `/api/sessions` at its path boundary and rejects other `/api` and `/api/*` paths. Only non-API paths use SPA fallback. An unavailable sessions upstream produces a proxy error, not SPA HTML.

With the stack already running, check database outage/recovery:

```sh
docker compose --env-file deploy/.env stop sessions-db
curl -i http://127.0.0.1:8080/api/sessions/healthz
curl -i http://127.0.0.1:8080/api/sessions/readyz
docker compose --env-file deploy/.env up -d --wait --wait-timeout 120 sessions-db
curl -i http://127.0.0.1:8080/api/sessions/readyz
```

Expect liveness 200 and readiness 503 during the outage, within readiness's 2-second total service timeout. Readiness should recover to 200 after PostgreSQL returns without restarting sessions (allow for an in-flight failed request).

## Direct-host development

Install the repository Rust toolchain and Node.js 24/npm. Use Compose only for the database; stop containerized web/sessions first. Alternatively provision the same database/role in host PostgreSQL 17 yourself; do not start Compose PostgreSQL on the same port.

```sh
docker compose --env-file deploy/.env stop web sessions
docker compose --env-file deploy/.env up -d --wait --wait-timeout 120 sessions-db
# Terminal 1: source only your trusted local file; Rust does not load .env.
set -a
. ./deploy/.env
set +a
export DATABASE_URL="postgresql://${POSTGRES_USER}:${POSTGRES_PASSWORD}@127.0.0.1:5432/${POSTGRES_DB}"
export HTTP_HOST=127.0.0.1 HTTP_PORT=3000 RUST_LOG=info
# Sourcing also exports any operator settings from deploy/.env; blank values keep the operator disabled.
cargo run --locked --package sessions-service --bin sessions-service
# Terminal 2:
(cd web && npm ci && npm run dev)
# Terminal 3:
curl -i http://127.0.0.1:3000/healthz
curl -i http://127.0.0.1:3000/readyz
curl -i http://127.0.0.1:5173/api/sessions/healthz
curl -i http://127.0.0.1:5173/api/sessions/readyz
```

Vite must bind strictly to `127.0.0.1:5173` and proxy to host sessions, not the Compose hostname. Frontend/sessions instructions own their code checks and startup behavior. Do not run host and Compose sessions simultaneously against the database when testing startup. Stop host processes with Ctrl-C; do not print the exported database URL.

## Stop and persistence

```sh
docker compose --env-file deploy/.env stop
# Restart:
docker compose --env-file deploy/.env up -d --wait --wait-timeout 120
# Remove containers/network, preserve data:
docker compose --env-file deploy/.env down
```

Named volume `sessions-db-data` stores PostgreSQL files at `/var/lib/postgresql/data`. Rebuilds, restarts, `stop`, and ordinary `down` retain it. Its name is fixed across checkouts: run one SansCue stack per machine and never attach the volume to multiple database processes. Use logical backups, not copies of live database files.

**Destructive reset: deletes the entire local database and migrations. Back up first. Never use this for ordinary shutdown:**

```sh
docker compose --env-file deploy/.env down -v
# Next up initializes an empty database; sessions reapplies migrations.
```

## Backup and restore

Keep backups private in ignored `local-data/`; they may contain future user data. Commands use the database image's PostgreSQL 17 clients, not host clients. Keep the same canonical role/database for restore.

The commands below cover the Sessions database only. Bee and Topics own
separate databases/volumes, so a complete product backup must also dump and
restore `bee-db` and `topics-db` independently. Those additional service
database backup/restore procedures have not yet been acceptance-tested.

Back up the running Sessions database:

```sh
mkdir -p local-data/backups
chmod 700 local-data local-data/backups
umask 077
backup="local-data/backups/sessions-$(date -u +%Y%m%dT%H%M%SZ).dump"
docker compose --env-file deploy/.env exec -T sessions-db sh -c 'pg_dump -U "$POSTGRES_USER" -d "$POSTGRES_DB" --format=custom' > "$backup"
# Check preceding exit status before trusting the file; inspect its catalog:
docker compose --env-file deploy/.env exec -T sessions-db pg_restore --list < "$backup"
```

Restore **overwrites matching database objects/data**. Stop all writers, including host-run sessions; back up current data before overwriting it. Choose a verified backup. Do not run `down -v` to perform a restore.

```sh
docker compose --env-file deploy/.env stop web sessions
docker compose --env-file deploy/.env up -d --wait --wait-timeout 120 sessions-db
backup=local-data/backups/REPLACE-WITH-VERIFIED-BACKUP.dump
docker compose --env-file deploy/.env exec -T sessions-db sh -c 'pg_restore -U "$POSTGRES_USER" -d "$POSTGRES_DB" --clean --if-exists --no-owner --no-privileges --single-transaction --exit-on-error' < "$backup"
# Only after successful restore:
docker compose --env-file deploy/.env up -d --wait --wait-timeout 120
curl -i http://127.0.0.1:8080/api/sessions/readyz
```

Restore preserves migration metadata; startup applies newer migrations. `--clean` replaces archived objects, not unrelated objects absent from the dump. For a full clean recovery use a separate empty local database/volume and a tested recovery plan rather than assuming this command wipes everything.

To check persistence without creating product tables, record existing migration metadata or a disposable local test marker, run ordinary `down` then `up --wait`, and confirm it remains. Back up, restore, and compare again. Remove disposable test objects afterward.

## Verification status

Operator Compose pass-through and documentation are covered by static checks (`python3 -B deploy/check-operator-config.py`), and the provisioning utility by stdlib unit tests, only; Docker/Compose behavior with and without operator values is unverified. Authored against the Stage 0 foundation contract. The authoring host has no Docker/Compose or Caddy executable; this isolated branch has no application source. Image builds, Compose config validation, `up --wait`, HTTP checks, persistence, and backup/restore execution therefore require integration and a Docker-enabled development machine. Static checks are not Stage 1 acceptance.
