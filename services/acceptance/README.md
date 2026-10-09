# Hardware-independent MVP acceptance

`synthetic-mvp.py` exercises the current end-to-end flow without Docker, a Bee
device, a model provider, or AWS. It starts the three Rust services locally,
uses the checked-in synthetic recovery fixture, and verifies binding delivery,
repeat replay idempotency, candidate evidence/staff-only visibility,
publication, an audience rating and written Q&A, response counts, and audience
dashboard privacy.

## Prerequisites

- Rust toolchain and Python 3.
- `psql` client.
- Three separately provisioned, disposable PostgreSQL 17 databases, with the
  current user allowed to connect and run migrations. They must be distinct:
  sessions `DATABASE_URL`, Bee `BEE_DATABASE_URL`, Topics
  `TOPICS_DATABASE_URL`. The script requires three distinct database names and
  refuses non-loopback database hosts to avoid accidental remote writes. It
  also rejects connection-string query overrides for `host`, `hostaddr`,
  `port`, `dbname`, or `service`; use the URL authority/path to specify the
  target. It never drops/truncates data. Use fresh databases for repeatable runs.
- Local ports 3300, 3301, and 3302 free.

Provision the databases outside this script, then export the URLs in the
current shell (do not commit them or put them in command-line arguments):

```sh
export DATABASE_URL='postgresql://USER:PASSWORD@127.0.0.1:5432/sanscue_sessions_test'
export BEE_DATABASE_URL='postgresql://USER:PASSWORD@127.0.0.1:5432/sanscue_bee_test'
export TOPICS_DATABASE_URL='postgresql://USER:PASSWORD@127.0.0.1:5432/sanscue_topics_test'
python3 services/acceptance/synthetic-mvp.py
```

The script creates a fresh operator credential and service tokens in process
memory, starts Sessions, Topics, and the Bee outbox worker with those ephemeral
values, and stops only those child processes. It does not emit the values or
resolved service environments. HTTP listeners are explicitly pinned to
`127.0.0.1`; services are launched as direct binaries (not Cargo wrappers) and
the runner verifies their process exit during cleanup. The replay creates fresh UUIDs on each run, so
previous test rows remain untouched. On success it prints one `PASS` line. A
missing database/service/tool or failed assertion is a failure; it is never
reported as skipped or passed. The script requires `psql` only to read the
speaker-session UUID associated with the freshly created room.

## What this does not establish

This is synthetic replay against local native PostgreSQL, not actual Bee stream
or API behavior. It exercises the `stub-v1` candidate path, not model quality.
It does not exercise Docker images/Compose/proxy, AWS, production reliability,
capacity, performance, backup/restore, or database outage recovery. Track those
as distinct gates in [environment verification](../../docs/environment-verification.md).

The runner's URL validation and listener-pinning safety checks do not need a
database and can be run separately:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest services/acceptance/test_synthetic_mvp.py
```
