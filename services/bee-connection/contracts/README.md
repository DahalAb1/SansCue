# Bee boundary v1

This crate owns its models and its own PostgreSQL database. It does not import
sessions business logic or read sessions tables. Existing-solution preflight:
reuse the repository's Serde, UUID, Chrono, Tokio and SQLx dependencies; there is
no existing Bee client or verified protocol in this repository. No speculative
SDK or new transport dependency is introduced.

## Internal room binding delivery (contract, not an HTTP endpoint)

An authenticated sessions producer must deliver a version-1 command containing
an internal SansCue conversation UUID, room UUID and session UUID. Sessions owns
speaker permission checks and room lifecycle authorization. Bee owns the stored
mapping. Retrying the same mapping is idempotent; changing an existing mapping
is rejected, so retries cannot move earlier transcripts into another room.
Use a new conversation UUID for a new source stream or session. UUIDs here do
not assert that Bee supplies any identifier. Unbound ingestion is quarantined,
not guessed, and is not silently reattached later.

Production transport/authentication, end-room/unbind delivery and transactional
outbox/inbox integration are deliberately not exposed yet. No public endpoint
accepts a room binding. This first implementation accepts trusted internal calls
and local sample input only. It is not a speaker authorization replacement.

## Canonical conversation event v1

The serialized Rust ConversationEvent is the canonical owned contract:
schema_version=1, internal event_id UUID, immutable binding, ingest_ordinal,
received_at (UTC), optional source_id/source_sequence, and text. The immutable
binding can carry an independently supplied source-conversation identity; when
present, every transcript observation must carry exactly the same identity.
If the source genuinely omits that identity, both values remain absent; the
pipeline does not substitute its internal UUID. Raw bytes are
retained separately for every observation, including malformed and duplicate
observations. Arrival ordinal is conversation-local and includes control,
quarantine and duplicate observations, so event ordinals need not be contiguous.
Arrival order is never described as speech order. Wall-clock times may regress.

Only a source-verified capability permits source-ID/sequence deduplication.
Capabilities apply to one immutable conversation stream: stable IDs cannot be
reused for revisions, and contiguous sequence numbers cannot reset or roll over.
If a real source differs, its adapter must map those semantics explicitly or
disable the capability; numeric metadata alone does not enable it.
Without identity evidence, identical text is two observations, not a duplicate.
The recording format is a SansCue synthetic fixture, NOT a Bee wire protocol.

Future delivery is at least once, keyed by internal event_id. Consumers must
idempotently accept schema version 1, retain binding and original ordinal, and
not assume arrival equals speech order. Acknowledgement must follow durable
consumer commit. Transactional outbound delivery is not implemented in this
branch; persisted events are not a claim of delivered jobs.

## External acceptance blocker

**REAL BEE DEVICE / LIVE API VERIFICATION**: authentication, exact raw payloads,
source identity scope, source ordering/reset semantics, reconnect behavior,
history/replay availability and recovery guarantees require real device/API
observations. Actual Bee-to-room acceptance is not complete. No claimed SDK,
endpoint, sequence field or recovery guarantee substitutes for that check.
