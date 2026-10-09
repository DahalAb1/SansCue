# Bee boundary v1

This crate owns its models and its own PostgreSQL database. It does not import
sessions business logic or read sessions tables. No real Bee protocol is
assumed by the canonical contract or binding worker.

## Sessions-authoritative room binding

The speaker-only sessions routes create immutable command intents and a durable
outbox in the sessions database. A command contains its stable command UUID,
monotonic room revision, operation (`bind` or `unbind`), internal conversation
UUID, room UUID, authorized speaker session UUID and an optional external source
conversation identifier. The optional source identity is only a value provided
at bind time; the integration does not infer one.

The authenticated private sessions listener exposes command polling,
acknowledgement and a binding-scoped Bee status-summary endpoint. Bee polls,
applies command plus inbox receipt atomically in its own database, delivers a
status summary, then acknowledges. Repeats are safe. Bee-local room revision
fences reject delayed older commands; a later bind closes an earlier local
binding when switching conversations. Sessions remains authoritative for
speaker permission and room lifecycle. Neither service queries the other's DB.

The current worker is device-independent; it is not a Bee transport. Exactly
one worker instance is supported because poll rows are not leased. Keep its
internal URL and service token on trusted private transport/network only; do not
expose the sessions private listener or token publicly. The repo's local Compose
file does not yet provision this service/credential. No public endpoint accepts
service credentials.

## Canonical conversation event v1

The serialized Rust `ConversationEvent` is the canonical owned contract:
`schema_version=1`, internal `event_id` UUID, immutable binding,
`ingest_ordinal`, `received_at` (UTC), optional `source_id`/`source_sequence`,
and text. The binding may carry a separately supplied source-conversation
identity; when present, transcript observations must match it exactly. If the
source omits identity, it remains absent; the pipeline does not substitute an
internal UUID. Raw bytes are retained for every observation, including malformed
and duplicate observations. Arrival ordinal is conversation-local, not speech
order; timestamps may regress.

Only source-verified capabilities enable source-ID/sequence deduplication.
Without identity evidence, identical text is two observations, not a duplicate.
The recording format and fixture are SansCue synthetic data, not a Bee wire
protocol. Real source identity scope, ordering/reset semantics, reconnect,
history and recovery guarantees still require device/API verification.
