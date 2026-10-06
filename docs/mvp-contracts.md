# MVP product contract

Agreed October 6, 2026 for local MVP implementation. The [MVP plan](mvp-plan.md), Steps 1–11, remains the original requirements and roadmap; the choices below are **implementation defaults**, not claims that the original plan specified them or that they are implemented. The [foundation contract](foundation-contract.md) remains authoritative for baseline ownership, ports, proxy routing, and health semantics. Its Stage 0 status statements are historical; see the [README](../README.md) for current status.

Implementation currently consists of the frontend shell, sessions health service, and local Compose foundation. Product behavior below is the target. Work is authorized toward `integration/mvp-complete` only, on assigned task branches: no merge into `dev`/`main`, push, or AWS deployment is implied. Participant history across sessions, native clients, automatic timers, and speculative scaling are deferred.

## Rooms, identity, and permissions

- Use opaque UUIDs for application entities and events; preserve external Bee IDs verbatim in separate source fields. IDs are not authorization.
- No account system. Creating a room establishes its speaker membership in a database-backed cookie session. Display the unique room join link as a QR code; invalid/revoked links return clear errors. An unguessable room join token grants audience membership only; separate unguessable, revocable invitations grant TA access. Rejoining with a valid cookie restores membership, not a duplicate participant. Never trust a role supplied by the browser.
- Cookies are HttpOnly, SameSite, and Secure under HTTPS; any HTTP exception is explicit and loopback-development-only. Validate origin and CSRF protection on mutations and origin/session/room authorization on WebSocket upgrades. Revocation applies to existing connections as well as subsequent requests. Do not log tokens or cookies.

| Role | MVP permissions |
|---|---|
| Speaker | Create/end the room, manage TA invitations/access and Bee binding, view dashboard and private Q&A, request generation, review and publish questions. |
| TA | Review/request generated candidates, manually publish questions, and view dashboard/private Q&A; cannot end rooms or manage access/Bee binding. |
| Audience | Join, view the active question, submit/change their own response, and submit/view their own written Q&A. No other participant's responses or Q&A. |

The speaker can operate alone. Audience response and written-Q&A tabs are separate; unfinished input survives tab changes. Ending a room is durable, broadcasts its ended state, and rejects new joins and participation, generation, and publication. Authorized existing members may read their permitted saved state.

## Questions, responses, and feedback

- Initial question format: a short understanding check about one evidenced topic, answered with exactly `clear`, `partly_clear`, or `need_help`. Topic precision means focusing each candidate on one coherent topic; it does not set publication frequency.
- Generation is explicitly requested by speaker/TA, asynchronous, and separate from audience requests. Use recent saved passages plus relevant earlier room context. Store candidate text, transcript evidence references, model ID, prompt/version/settings, and generation time. Unrelated or insufficient evidence yields a visible failure, not an invented question.
- Publication is manual by speaker/TA. There is at most one active published question per room. Publishing a candidate transactionally replaces the active pointer, not the previous published record. A published copy is immutable, including its evidence references. Late generation results remain candidates and never replace the active question automatically. No automatic publication or answer timers.
- One response per audience membership and published question. Changing an answer replaces the prior selection atomically; it never adds a second counted response. Reject responses for inactive questions or ended rooms.
- Written Q&A is private: visible only to its author and room speaker/TAs, not a public audience feed. Save the room, optional current published-question reference, and available context/evidence references at submission. No public moderation feed or threaded replies in this MVP.
- Dashboard shows active question, distinct respondent count, counts and percentages for each option (denominator: respondents; zero respondents means no rating), and context-linked written Q&A. No inferred comprehension score or automatic attention alarm.

## API, live state, and delivery

- Browser HTTP and WebSocket access stays same-origin under `/api/sessions`; sessions is the browser-facing backend. Preserve `/healthz`, `/readyz`, their exact bodies/statuses/timeouts, and proxy prefix stripping from the foundation contract. Unknown API paths remain 404, never SPA HTML.
- Product mutations require `Idempotency-Key` (a client-generated UUID retained across retries). Scope stored keys to the authenticated actor and operation/room; bootstrap create/join requests bind keys to the originating browser session. Persist the request fingerprint and outcome atomically with effects. Same key/body returns the original outcome; changed body returns 409. Check current authorization even on replay.
- Product API errors use JSON `{"code":"stable_code","message":"safe explanation","request_id":"uuid"}` with appropriate HTTP status (400 invalid input, 401 missing session, 403 forbidden, 404 missing resource, 409 state/idempotency conflict, 503 unavailable dependency). Health responses remain unchanged; proxy-generated errors need not use this body. Never expose secrets or raw database/provider errors.
- Commit state and its outgoing event before acknowledgement/delivery. Event envelope: `event_id` (UUID), `event_type`, `schema_version` (initially 1), `room_id`, `occurred_at` (producer time), and `payload`; sessions live events also carry a durable per-room `sequence`. Producer time is not Bee speech time.
- Sessions orders room changes by sequence. Reconnect restores an authorized snapshot with its sequence, then only later events; buffer/replay across snapshot acquisition or force resnapshot on a gap. Do not silently lose changes during recovery. Filter both snapshots and events by role/ownership. Bound socket queues; disconnect slow clients and resynchronize.
- Use authenticated internal HTTP delivery for this MVP, with service credentials kept server-side, never in browser bundles. Commit a durable outbox with producing changes; consumers deduplicate by `event_id` transactionally. Retry transient failures with capped backoff (maximum five automatic attempts per delivery); persist exhausted failures for inspection/explicit retry. Duplicates/reordering must not roll back newer state. Internal delivery may repeat and exhausted deliveries remain failed; this does not guarantee eventual receipt or change Bee’s delivery guarantees.

## Service and data ownership

| Backend | Owns database, migrations, and durable state |
|---|---|
| `sessions` | Rooms, tokens/invitations, cookie sessions/memberships, published copies, responses, private Q&A, room sequence/events, idempotency records. |
| `bee-connection` | Bee conversation-to-room bindings, raw events, receipt metadata, connection/gap records, transcript delivery outbox. |
| `topics-and-questions` | Received context/evidence references, generation jobs, candidates, model/prompt metadata and evaluation results, candidate delivery outbox. |

Each backend has one separate database and service-owned credentials/pool; one PostgreSQL server may host them locally. No cross-database reads or shared business-model crate. Each service owns its delivery/dedup records. Sessions authorizes binding changes and forwards them through authenticated internal APIs; Bee owns the authoritative binding. Exact endpoint/schema files belong to their implementing owners and must conform to this contract. No broker or replicas are required.

## Bee boundary and model evidence

The researched local proxy exposes unauthenticated SSE `GET /v1/stream` on `127.0.0.1:8787`. It is **local-only and must never be exposed publicly**, proxied through Caddy, or sent directly to browsers. The adapter must use an explicit private local connection; do not change the proxy to a public bind to solve container reachability. See [Bee realtime documentation](https://docs.bee.computer/docs/realtime).

The stream is at-most-once. Its documented sample has `utterance.text`, `utterance.speaker`, and `conversation_uuid`, but no documented replay cursor, stable utterance ID, source timestamp, or speech ordering guarantee. Store every raw event with a local event UUID, received time, and monotonic arrival index per binding, preserving external IDs. Do not deduplicate utterances by text or call arrival order speech order. Record disconnect/reconnect gaps; missed source events may be unrecoverable. Internal retries can recover delivery of already stored events, not missing Bee speech. Unknown/unbound conversations must not be routed to an arbitrary room.

Recorded/synthetic fixtures support deterministic tests and must be labeled; they do not prove actual Bee ingestion. Bee account/authentication, device access, and recording consent require human involvement. Do not commit private recordings.

Model/provider selection remains open: compare two candidates on identical consented Bee excerpts and instructions, recording exact model IDs, prompt/settings, evidence quality, complete-question delay, and cost. Agree targets before evaluation; fixtures/mock output do not select a production model. No AWS credentials or paid model spend is assumed or authorized.

## Local verification and completion evidence

Use [local runtime instructions](../deploy/README.md) for configuration/startup/backup/restore and [sessions verification](../services/sessions/tests/README.md) for database prerequisites. Branch-isolation statements in the runtime document describe its original authoring context, not the integrated tree. Baseline commands are:

```sh
(cd web && npm ci && npm run typecheck && npm run build)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
docker compose --env-file deploy/.env config --quiet
docker compose --env-file deploy/.env up --build --wait --wait-timeout 120
python3 services/sessions/tests/outage-recovery.py
python3 deploy/check-build-context.py
```

Supply a disposable PostgreSQL 17 test database through `DATABASE_URL`; Rust does not load dotenv implicitly. Compose needs a locally populated `deploy/.env`, Docker/Compose and registry access. Never publish resolved secrets. These are required checks, **not reported passes**. The existing Compose stack contains only the foundation; implementation must add the two backend services/databases and document their local configuration without exposing Bee.

Acceptance also requires automated and browser checks for QR joining/invalid links, role isolation/revocation/CSRF, idempotent retries and conflicting payloads, concurrent answer replacement/counts, immutable publication/late candidates, private Q&A, reconnect race/gap recovery, room ending, duplicate/out-of-order internal delivery and exhausted retries, Bee gap visibility, evidence-linked generation, and backup/restore. Verify direct/Vite/Caddy health and unknown API routes, including real database outage/recovery. Record exact tested commits and unexecuted checks.

Actual Bee ingestion, two-model evaluation, agreed workload/delay/cost targets and measurements remain explicit acceptance items; unavailable access is a blocker, not a pass. Local implementation is not AWS deployment or full MVP completion. See [development context](development-context.md) for workflow and [first-iteration context](agent-iteration-1.md) for historical direction.
