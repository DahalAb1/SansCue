# Local development runtime

Local-only packaging of the static Preact/Vite build behind Caddy, sessions, and PostgreSQL. Run commands from the repository root. This is not a production configuration.

## Prerequisites and integration

- Docker Engine/Desktop with a running daemon and Compose v2 supporting `up --wait --wait-timeout`; host `curl` for checks.
- Integrate frontend and sessions before building. This branch alone has no application source. Web expects `web/package.json`, `web/package-lock.json`, `npm run build`, and output `web/dist/`. Sessions expects root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, and Cargo package/binary `sessions-service` under `services/sessions/`. The service must embed its migrations at build time.
- Free loopback ports 8080 and 5432; host development also needs 3000 and 5173.
- Initial builds need image/package registry access. Official Node, Caddy, Rust, Debian, and PostgreSQL images provide the runtime; no custom orchestration.

Both Dockerfiles use repository-root build contexts. Root `.dockerignore` defaults to deny and allows only the current manifests, build configuration, Rust/SQL source, and TypeScript/TSX/CSS source. New source or asset types require an explicit allowlist update; do not reinclude whole application directories. Sensitive directories and credential filenames are also denied after the allowlist. Terminal descendant exclusions for every allowed path prevent extension-shaped or manifest-named directories from admitting arbitrary children; keep these exclusions last when extending the allowlist. Run `python3 deploy/check-build-context.py` with Docker/BuildKit to verify 17 required inputs, 176 harmless credential/generated fixture paths, and 150 descendants of source-shaped or manifest-named directories using two isolated scratch builds (no actual credentials or application directories are copied). No database configuration enters the browser/web build. Application images explicitly install `curl` for probes. PostgreSQL 17 includes `pg_isready`, `psql`, `pg_dump`, and `pg_restore`.

## Configure and start

```sh
cp deploy/.env.example deploy/.env
chmod 600 deploy/.env
# Edit deploy/.env locally; never commit it.
docker compose --env-file deploy/.env config --quiet
docker compose --env-file deploy/.env up --build --wait --wait-timeout 120
docker compose --env-file deploy/.env ps
docker compose --env-file deploy/.env logs --tail=100 web sessions sessions-db
```

Keep database `sessions` and local role `sessions_app`. The sample password is local-only and not production-ready. Use only URL-unreserved password characters (letters, digits, `. _ ~ -`): Compose interpolates it directly into the private sessions `DATABASE_URL`. Do not export competing `POSTGRES_*` shell values: those override the env file. If needed, inspect `docker compose --env-file deploy/.env config` only locally. **Its resolved output contains credentials; never publish it, container environment inspection, or credentials in logs/issues.**

PostgreSQL creates the database and role on first initialization only. Its `POSTGRES_USER` is a local bootstrap superuser, not a least-privilege production role design. Changing `POSTGRES_*` does not update an existing database/password. Sessions owns and applies migrations and its pool; runtime creates no product tables or separate migration runner.

- Browser: `http://127.0.0.1:8080`; Caddy listens on container `:80`.
- Sessions: `sessions:3000` internally, bound to `0.0.0.0:3000`; no published sessions host port.
- PostgreSQL: `sessions-db:5432` internally, `127.0.0.1:5432` on the host.

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

Back up the running database:

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

Authored against the Stage 0 foundation contract. The authoring host has no Docker/Compose or Caddy executable; this isolated branch has no application source. Image builds, Compose config validation, `up --wait`, HTTP checks, persistence, and backup/restore execution therefore require integration and a Docker-enabled development machine. Static checks are not Stage 1 acceptance.
