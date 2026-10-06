# Foundation contract

Agreed October 6, 2026. This is the shared implementation agreement for Stage 1 frontend, sessions service, and local runtime work, not a description of existing code. Stage 0 adds documentation only; Stage 1 is incomplete.

The [MVP plan](mvp-plan.md) retains the broader product and AWS deployment goals. This agreement narrows the immediate foundation work to a local development/runtime baseline; it does not complete MVP Step 1 or authorize AWS deployment. Rooms, authentication, Bee, AI, and other future features are out of scope.

## Repository boundaries and ownership

Use one repository with these planned boundaries. Create files only when implemented; Stage 0 creates no code directories, placeholders, manifests, or future services.

| Owner | Stage 1 responsibility |
|---|---|
| Frontend branch | `web/`: private npm package (`private: true`), Preact + TypeScript + Vite + CSS; package/lockfile, configuration, index, source, and Vite proxy. |
| Sessions branch | Root Cargo workspace, lockfile and toolchain; independently runnable Rust + Axum + Tokio + PostgreSQL + SQLx service at `services/sessions/`; configuration, startup, health checks, migrations, tests, and `services/sessions/.env.example`. Technical short name: `sessions`. |
| Runtime branch | Root `compose.yaml`, `deploy/` local runtime files (`.env.example`, `Caddyfile`, `README.md`), Dockerfiles, and `.dockerignore`. |
| Foundation branch | This contract, the root README, and the implementation-agreement cross-reference in `docs/development-context.md` only. |

There is no root JavaScript workspace or shared business-model crate. The service owns its SQLx pool and migrations; runtime provisions its database and role. Do not scaffold later services or their routes.

## HTTP and proxy agreement

Browser requests are same-origin; no CORS setup is needed. Both Vite and Caddy strip exactly the `/api/sessions` prefix before forwarding to sessions:

| Browser GET | Service GET | Response |
|---|---|---|
| `/api/sessions/healthz` | `/healthz` | `200`, `application/json`, `{"status":"ok"}`; no database dependency. |
| `/api/sessions/readyz` | `/readyz` | Run `SELECT 1` through the service-owned SQLx pool. Return `200`, `application/json`, `{"status":"ready"}` when reachable; otherwise `503`, `application/json`, `{"status":"not_ready"}`. |

Readiness has a **2-second total timeout**, including pool acquisition and the query. Never include database errors or credentials in its response.

Unknown backend paths return `404`. Unmatched `/api/*` requests must not fall through to SPA HTML in either Vite or Caddy. This contract defines no other APIs, later-service routes, or error-response bodies.

If database initialization or migrations fail before the listener binds, startup is bounded and the process exits nonzero; the sessions implementation must choose and document that startup bound. After binding, database failure leaves liveness at `200` and readiness at `503`; readiness recovers when the database returns without requiring a service restart.

## Service configuration

| Setting | Agreement |
|---|---|
| `DATABASE_URL` | Required, private service configuration; never exposed to the browser. |
| `HTTP_HOST` | Default `127.0.0.1`; Compose sets `0.0.0.0`. |
| `HTTP_PORT` | Default `3000`. |
| `RUST_LOG` | Default `info`. |
| SQLx pool | Maximum 5 connections; acquisition timeout 5 seconds. The stricter 2-second total readiness timeout still applies. |

Rust does not implicitly load `.env` files. Supply settings explicitly through the process environment or Compose. Example files contain placeholders/local-development values only; no browser secrets. `POSTGRES_*` settings belong to Compose database provisioning, not the service configuration API. Runtime constructs the container `DATABASE_URL` using hostname `sessions-db`.

## Local runtime and database

| Endpoint | Binding/address |
|---|---|
| Vite development server | `127.0.0.1:5173`, strict port (do not silently choose another port). |
| Host-run sessions | `127.0.0.1:3000`. |
| Host PostgreSQL mapping | `127.0.0.1:5432`. |
| Compose web/Caddy host mapping | `127.0.0.1:8080`; Caddy listens on `:80` inside its container. |
| Container-to-container sessions | `sessions:3000`. |
| Container-to-container PostgreSQL | `sessions-db:5432`. |

Vite proxies to host-run sessions; Caddy proxies to container sessions. These bindings are for local development, not LAN hosting or an AWS deployment.

Use PostgreSQL major **17**, database **`sessions`**, local role **`sessions_app`**, and persistent volume **`sessions-db-data`**. Runtime provisions the database/role and documents local configuration and persistence. Sessions owns and applies its migrations and uses its own connection pool. No product tables are defined now; the sessions branch chooses the minimal bootstrap migration.

## Planned Stage 1 commands and checks

**Not currently runnable:** package files, Rust code, Compose, and environment examples do not exist in Stage 0. These are downstream verification targets, not current setup instructions. Owners must provide their prerequisites and runnable instructions with implementation.

Frontend (after its package and lockfile exist):

```sh
cd web
npm ci
npm run typecheck
npm run build
```

Sessions (from the repository root, with the documented test database/environment):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Runtime (from the repository root, after populating the planned local environment file):

```sh
docker compose --env-file deploy/.env config
docker compose --env-file deploy/.env up --build --wait --wait-timeout 120
curl -i http://127.0.0.1:8080/api/sessions/healthz
curl -i http://127.0.0.1:8080/api/sessions/readyz
```

Do not publish resolved Compose configuration containing secrets. The 120-second Compose wait is a verification bound, not the service startup timeout. With the separate host development processes running, also check:

```sh
curl -i http://127.0.0.1:3000/healthz
curl -i http://127.0.0.1:3000/readyz
curl -i http://127.0.0.1:5173/api/sessions/healthz
curl -i http://127.0.0.1:5173/api/sessions/readyz
```

## Acceptance by stage

### Stage 0 — documentation foundation

- This single contract records the agreed boundaries, owners, routes, health semantics, configuration, and local runtime targets.
- README links here and remains honest about Planning status and absent runnable code. Development context links to this implementation agreement without duplicating it.
- Only `docs/foundation-contract.md`, `README.md`, and `docs/development-context.md` change; `git diff --check` passes.
- No product implementation, placeholder directories, deployment, or Stage 1 completion claim.

### Stage 1 — pending implementation and integration

- Frontend shell, independently runnable sessions service, database/bootstrap migration, and local Compose runtime exist under the assigned ownership boundaries; planned checks pass.
- Direct, Vite-proxied, and Caddy-proxied health/readiness requests match this contract; the browser uses relative same-origin URLs.
- Unknown backend paths return `404`; unmatched `/api/*` never returns SPA HTML through either proxy.
- Tests/checks cover readiness's total timeout, unavailable database, bounded pre-bind database/migration failure with nonzero exit, and post-bind database outage/recovery while liveness remains healthy.
- Configuration defaults, required private database URL, strict Vite port, loopback host bindings, and PostgreSQL persistence are verified and documented.
- These checks establish only the local foundation. AWS deployment and broader MVP Step 1 acceptance remain separate and incomplete.
