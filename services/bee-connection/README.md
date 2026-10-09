# Bee connection: canonical ingest and command worker

Independent Rust executable/library, with a Bee-owned PostgreSQL database. It
never reads sessions tables or shares business models. Its device-independent
worker polls the authenticated sessions command outbox and applies binding
commands via the Bee store. It also forwards newly accepted canonical events
from the Bee-owned transactional outbox to Topics. See the [v1 contract](contracts/README.md). **REAL BEE DEVICE /
LIVE API VERIFICATION** remains required; no actual-device acceptance is claimed.

## Synthetic replay

Provision a separate PostgreSQL 17 Bee database and dedicated role externally.
Never point Bee settings at the sessions database. Supply the URL privately
through `BEE_DATABASE_URL`; no dotenv loading. To deliver replay events through
the candidate pipeline, also run the `--worker` process with `TOPICS_INTERNAL_URL`
and `TOPICS_BEE_SERVICE_TOKEN` configured; no hardware transport is started.
From the repository root:

~~~sh
cargo run -p bee-connection -- services/bee-connection/fixtures/synthetic-recovery.json 00000000-0000-0000-0000-000000000001 00000000-0000-0000-0000-000000000002 00000000-0000-0000-0000-000000000003
~~~

Arguments are recording, internal conversation UUID, room UUID and session UUID.
This is a trusted local replay harness, not a Bee transport. The fixture is
synthetic and not a recording of real Bee traffic.

## Device-independent binding worker

Configure `BEE_DATABASE_URL` for the Bee-only database, `SESSIONS_INTERNAL_URL`
to the private sessions listener (for example `http://sessions:3001`), and
`SESSIONS_BEE_SERVICE_TOKEN` to the credential configured in sessions,
`TOPICS_INTERNAL_URL` to the private Topics listener, and
`TOPICS_BEE_SERVICE_TOKEN` to the credential configured in Topics. The sessions
private listener must bind an address reachable from the worker (set
`SESSIONS_INTERNAL_HOST=0.0.0.0` in a container network). Run:

~~~sh
cargo run -p bee-connection -- --worker
~~~

Exactly one worker instance is supported for now; commands are not leased, so
multiple workers may race delivery/acknowledgement. Keep the sessions internal
URL and service token on trusted private transport/network only. Never expose
the private listener or token on a public route/network. The local Compose file
now provisions the worker and its separate Bee database, but does not start or
claim any Bee device transport.

The worker polls sessions commands, atomically applies each command and its
idempotency receipt in Bee PostgreSQL, posts a binding status summary, then
acknowledges the sessions outbox row. Stale revisions are acknowledged without
publishing old status, so a stale head command cannot starve later work. It
also polls Bee's event outbox and acknowledges an event only after Topics
confirms receipt. Explicit invalid/conflicting event callbacks are marked
terminal with their machine-readable rejection code; transient errors remain
retryable, and one terminal row does not block later rows. The worker starts no
Bee/device transport. Its candidate path requires an authoritative room binding
made through Sessions; the local replay CLI's standalone binding is only a
Bee-store harness.

## Storage and gap semantics

The first-step store uses a locked JSONB aggregate per conversation. Binding,
raw bytes, decode errors, canonical events, ordinal and gap evidence commit in
one transaction. A newly accepted event and its Topics outbox entry commit in
that same transaction. Command inbox receipts and per-room revision fences make
sessions commands idempotent and reject delayed old commands. The aggregate is
suitable for bounded samples, not measured production streams; production needs
bounded retention/input policies and normalized append storage.

A disconnect records a suspected interval; reconnect alone does not establish
completeness. Only verified contiguous source sequences can confirm missing
ranges. No sequence evidence means suspected incompleteness, not a known lost
event count. The synthetic fixture does not establish real Bee identity, order,
reconnect or recovery behavior.

## Verification

~~~sh
cargo fmt --all -- --check
cargo test -p bee-connection --lib --test contract --test pipeline
cargo test -p bee-connection --test persistence
cargo test -p sessions-service --no-run
~~~

Persistence needs `BEE_TEST_DATABASE_URL` for a disposable Bee-only DB; the
sessions integration suite needs `DATABASE_URL` for a separate sessions test DB.
Docker and real-device/API verification remain separate.
