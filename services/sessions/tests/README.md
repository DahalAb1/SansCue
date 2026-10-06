# Sessions verification

From the repository root, provision a disposable PostgreSQL 17 database and supply
`DATABASE_URL` explicitly (Rust does not load dotenv files), then run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The regular integration suite requires that database and fails rather than silently
skipping DB checks. It verifies migrations, pool-acquisition timeout/recovery,
unavailable database behavior, and bounded pre-bind connection/migration failure.
Startup has a 15-second total bound; readiness has a 2-second total bound. Releasing
held pool connections is **not** proof of actual database outage recovery.

## Real post-bind database outage/recovery

Additional prerequisites: Python 3, Docker Engine with a working daemon, permission
to create containers, registry access to `postgres:17`, and the repository Rust
toolchain. Run separately (not implicitly by `cargo test`):

```sh
python3 services/sessions/tests/outage-recovery.py
```

This automated check builds the host service, creates its own uniquely named
PostgreSQL 17 container with a random loopback port and data in its disposable
writable layer (retained across restart, removed during cleanup), and
starts sessions against it. It ignores the caller's `DATABASE_URL` and never stops
an existing DB or Compose stack. After verifying HTTP readiness, it kills the real
PostgreSQL container, checks liveness stays 200 and readiness becomes 503 with exact
JSON bodies (under 3 seconds including transport/scheduling tolerance), then starts
the same DB container and requires readiness recovery within 30 seconds without
restarting sessions. The same child process must remain alive throughout. It
cleans up only its own service process and DB container on success or failure.
The temporary DB is intentionally not a persistence test. Do not use `python -O`,
which disables assertions.

Missing Docker/DB infrastructure is a failing prerequisite, not a skipped or
passing test. Route it to the active environment Fixer; do not substitute the
pool-release test or weaken the foundation contract. On the authoring host Docker
was unavailable, so this check still requires execution in the repaired environment.
