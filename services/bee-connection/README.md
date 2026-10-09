# Bee connection: core boundary and synthetic replay

Independent Rust executable/library, with a Bee-owned PostgreSQL database. No
sessions DB reads, shared business-model crate, public routes, speaker access
endpoints, live Bee transport or outbox integration. See the [v1 delivery
contract](contracts/README.md). **REAL BEE DEVICE / LIVE API VERIFICATION** remains
required; this is not complete actual-device acceptance of MVP Step 7.

## Run the local sample

Provision a separate PostgreSQL 17 database and dedicated Bee role externally.
Never point Bee settings at the sessions database: migration histories are
service-owned. Supply the URL privately through the environment; no dotenv
loading. Startup connection/migrations have a total 15-second bound, with at
most 5 pooled connections. Do not commit actual recordings or credentials.

From repository root, with BEE_DATABASE_URL set:

~~~sh
cargo run -p bee-connection -- services/bee-connection/fixtures/synthetic-recovery.json 00000000-0000-0000-0000-000000000001 00000000-0000-0000-0000-000000000002 00000000-0000-0000-0000-000000000003
~~~

Arguments are recording, internal conversation UUID, room UUID, session UUID.
This is a trusted local harness, not an authenticated API. It opens the mapping,
replays all input, persists each observation before proceeding and prints counts
only (never transcripts/credentials). Replaying appends raw retry observations;
verified stable IDs suppress duplicate canonical events. Control observations
are retained on every run. Use a new conversation UUID for independent runs.
The checked-in fixture is entirely synthetic, including raw byte samples and
source ID/sequence capabilities; it is not a recording of real Bee traffic.

## Storage and gap semantics

The first-step store uses one locked JSONB aggregate per conversation. Binding,
raw bytes, decode errors, canonical events, ordinal and gap evidence commit in
one transaction. Concurrent writers serialize by row lock. Reload restores IDs,
duplicate suppression and open gaps. A failed/ambiguous acknowledgement may be
retried; only verified identity permits duplicate suppression. No ID evidence
means at-least-once raw arrival, not an exactly-once transcript claim.

This deliberately simple aggregate rewrites and loads the full conversation;
it is suitable for bounded samples, not measured production streams. Production
needs bounded input/retention policy, normalized append storage, query/delivery
indexes, cancellation/timeouts, async adapter backpressure and authenticated
binding/lifecycle delivery before deployment. Do not deploy this local harness
as a live service. Database constraints identify conversations; per-conversation
rules are protected by the transactional row lock, not cross-service SQL.

A disconnect records a suspected interval with a receive time and arrival
ordinal; reconnect records its end but alone does not establish completeness.
Verified contiguous source sequences can confirm a missing inclusive range;
all missing sequence values must arrive before marking it recovered. An adjacent
post-reconnect sequence proves that bounded interval had no missing records.
This says nothing about history preceding the first observed sequence. Missing
sequence metadata leaves a separate suspected gap. No sequence evidence means
suspected forever unless explicitly marked unrecoverable. RecoveryUnavailable
marks all unresolved gaps terminal with its reason; late arrivals do not silently
rewrite that declaration. Numeric jumps and disconnect intervals may overlap;
they are evidence records, not a count of unique lost passages.

## Checks

~~~sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p bee-connection --test contract --test pipeline
cargo test -p bee-connection --test persistence
cargo test --workspace
~~~

Persistence requires BEE_TEST_DATABASE_URL for a disposable **Bee-only** DB;
workspace tests also require DATABASE_URL for a separate sessions test DB. Missing
DB settings fail loudly, not skip. Persistence tests cover repeat migrations,
concurrent deduplication, restart equality, immutable binding, raw quarantine,
arrival continuation and persisted suspected gaps. Real process/database outage,
backup/restore, container builds and live-device recovery are separate checks.

### Authoring verification (October 9, 2026)

- Formatting and strict workspace Clippy passed.
- Focused contract/replay tests: 3 passed; pipeline tests: 15 passed.
- Sessions library regression tests: 6 passed.
- Bee persistence test and full workspace suite were attempted and failed on the
  required missing BEE_TEST_DATABASE_URL (not skipped). DATABASE_URL was also
  absent. Native PostgreSQL accepts connections, but the current OS role has no
  database role and postgres peer authentication is unavailable to this process.
  No separate Bee test database credentials were provided; no sessions database
  was repurposed and no database/credential infrastructure was changed.
- Static Docker allowlist validation passed: 21 allowed fixture inputs and 180
  terminal-descendant fixture cases. Actual Docker context/build verification
  failed its prerequisite: Docker with BuildKit is unavailable.
- Standalone executable rejects missing arguments, invalid UUIDs and missing
  BEE_DATABASE_URL with exit status 1 and non-sensitive messages. Successful
  PostgreSQL-backed sample execution remains pending DB provisioning.

Persistence code and tests compile, but this record does **not** claim successful
PostgreSQL persistence, concurrency, migrations or restart verification. Supply
the two separate disposable test database URLs and rerun the exact checks above
before integration approval.
