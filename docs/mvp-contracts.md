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

Each backend has one separate database and service-owned credentials/pool; one PostgreSQL server may host them locally. No cross-database reads or shared business-model crate. Each service owns its delivery/dedup records. Sessions authorizes binding changes and forwards them through authenticated internal APIs; Bee owns the authoritative binding. Implementing owners may add schema files but must preserve the canonical interfaces below. No broker or replicas are required.

## Canonical interfaces (chosen MVP conventions)

These wire contracts are implementation defaults, not implemented APIs. JSON uses snake_case, local UUID strings, RFC 3339 UTC timestamps, and nonnegative safe JSON integers for revisions. Examples use R/M/Q/J/C/B/E/I/T as placeholders for room/membership/published question/job/candidate/binding/event/invitation/time, not literal valid values. Reject unknown mutation fields and invalid enums with 400. Product responses are JSON with Cache-Control: no-store. Sessions retains its implemented **15-second total startup bound** and foundation **2-second total readiness bound**.

### Browser security and idempotency

All paths below are relative to /api/sessions. S = speaker, T = TA, A = audience, B = bootstrap session. Every room/entity route checks membership in that exact room; inaccessible entities return 404, insufficient role in an accessible room returns 403.

- GET /bootstrap creates/restores a database-backed anonymous browser session and returns 200 {"csrf_token":"opaque"}. Cookie: sanscue_session, HttpOnly, Path=/, SameSite=Lax, Secure under HTTPS; only explicit loopback development permits HTTP. Use a 12-hour idle and 24-hour absolute server-enforced session lifetime (cookie Max-Age=86400); expiry requires fresh bootstrap and a valid join/invite or new room, with no anonymous recovery of speaker privileges. Keep session identity stable through join/create/redeem for lost-response retries. Roles are resolved server-side. Bootstrap is not an idempotency-keyed product mutation.
- Every mutation requires that cookie, Content-Type: application/json, X-CSRF-Token bound to the session, Idempotency-Key (UUID), and exact configured same-origin Origin. Missing/incorrect Origin or CSRF is 403; no wildcard CORS. WebSocket upgrades require cookie, membership and exact Origin; no secrets in query strings. Revocation and session expiry take effect on requests and delivery, closing affected sockets with 1008; enforce expiry even on idle sockets. Idle lifetime is refreshed by authenticated HTTP activity, not server-pushed frames.
- Idempotency scope: browser-session ID + method + canonical target path (including room/entity/token identity). Fingerprint normalized JSON and target, not CSRF headers. Persist successful status/body with effects/outbox atomically; same key/body replays that outcome, changed body returns 409 idempotency_conflict. Concurrent duplicates wait or return 409 request_in_progress for retry with the same key. Retain records as long as session/room data; no time expiry in MVP. Failures before commit do not reserve a key. Current authorization/revocation checks still apply on replay; a consumed invitation can replay only for its original redeemer. A committed success can replay after room end; new effects cannot.
- Remote commands use a durable local command ID/outbox before returning 202; acceptance is not remote completion. Retrying cannot produce another remote effect. Reads do not mutate product state. Never log tokens/cookies/CSRF values or token-bearing paths.
- Error shape: {"code":"room_ended","message":"Room has ended","request_id":"uuid"}. Status/code pairs: 400 invalid_request; 401 session_required; 403 forbidden/csrf_failed/origin_denied; 404 not_found (including invalid/revoked tokens); 409 room_ended/question_inactive/idempotency_conflict/request_in_progress/invite_used/binding_conflict; 503 dependency_unavailable. Job failures are job states, not candidate successes. Existing health/proxy exceptions remain unchanged.

### Browser routes

State, Published, Candidate, Job and Written below name JSON objects, not strings. Empty mutation bodies are {}. List routes accept `?after={opaque_cursor}&limit=50` (1–100) with stable creation-order + ID pagination and next_cursor:null at end. Sessions forwards candidate cursors to topics unchanged.

| Method/path | Role | Request JSON | Success status/body |
|---|---|---|---|
| `POST /rooms` | B | {"title":"Talk"} | 201 {"state":State,"join_url":"/join/{token}"}; creator becomes S |
| `POST /join/{token}` | B | {} | 200 {"state":State}; create/reuse A, never downgrade S/T |
| `GET /rooms/{id}/state` | S/T/A | — | 200 State |
| `POST /rooms/{id}/invitations` | S | {} | 201 {"invitation_id":"I","invite_url":"/invite/{token}","status":"pending","expires_at":"T"} |
| `GET /rooms/{id}/invitations` | S | — | 200 {"items":[{"invitation_id":"I","status":"pending","membership_id":null,"expires_at":"T"}],"next_cursor":null} |
| `POST /invitations/{token}/redeem` | B | {} | 200 {"state":State}; create/upgrade T |
| `POST /rooms/{id}/invitations/{invitation_id}/revoke` | S | {} | 200 {"invitation_id":"I","status":"revoked"} |
| `POST /rooms/{id}/memberships/{membership_id}/revoke` | S | {} | 200 {"membership_id":"M","status":"revoked"}; T/A only |
| `POST /rooms/{id}/join-link/rotate` | S | {} | 200 {"join_url":"/join/{token}","join_link_enabled":true} |
| `POST /rooms/{id}/join-link/revoke` | S | {} | 200 {"status":"revoked"}; stops new joins, not existing members |
| `POST /rooms/{id}/generations` | S/T | {} | 202 {"job_id":"J","status":"queued"} |
| `GET /rooms/{id}/generations` | S/T | — | 200 {"items":[{"job":Job,"delivery":{"status":"pending","failure":null}}],"next_cursor":null} |
| `GET /rooms/{id}/candidates` | S/T | — | 200 {"items":[Candidate],"next_cursor":null} |
| `POST /rooms/{id}/questions` | S/T | {"candidate_id":"C"} | 201 {"question":Published,"revision":7} |
| `GET /rooms/{id}/questions` | S/T/A | — | 200 {"items":[Published],"next_cursor":null}; includes archived copies |
| `POST /rooms/{id}/questions/{question_id}/close` | S/T | {} | 200 {"active_question":null,"revision":8}; Q must be active |
| `PUT /rooms/{id}/questions/{question_id}/response` | A | {"selection":"clear"} | 200 {"question_id":"Q","selection":"clear","updated_at":"T","revision":9}; submit/change |
| `POST /rooms/{id}/written-questions` | A | {"text":"Please explain X","published_question_id":null} | 201 Written |
| `GET /rooms/{id}/written-questions` | S/T/A | — | 200 {"items":[Written],"next_cursor":null}; A sees only own |
| `PATCH /rooms/{id}/written-questions/{written_id}/status` | S/T | {"status":"addressed"} | 200 Written; status is open or addressed |
| `POST /rooms/{id}/end` | S | {} | 200 {"room_id":"R","status":"ended","active_question":null,"revision":10} |
| `GET /rooms/{id}/live?after=9` | S/T/A | WebSocket upgrade; optional last applied sequence | 101; frames below |
| `PUT /rooms/{id}/bee-binding` | S | {"conversation_uuid":"external-value"} | 202 {"command_id":"E","status":"pending"} |
| `DELETE /rooms/{id}/bee-binding` | S | {} | 202 {"command_id":"E","status":"pending"} |

Trim outer whitespace; title is 1–200 characters, written text 1–4000. Join/invite tokens have at least 256 random bits and are stored only as SHA-256 hashes; return plaintext links only in their issuance response. Chosen narrow replay exception for hash-only storage: token-issuing mutations persist the outcome with link fields null; replay returns the original status and IDs but link fields null and link_unavailable:true, without issuing another token. The browser retains the original link in memory; if its response was lost, use a new-key rotation or a new invitation (revoke the old invitation). All other mutations replay the original body exactly. Link landing pages use Referrer-Policy: no-referrer; redemption is never GET. Links are frontend routes. Invitations are single-use and expire 24 hours after issuance; statuses pending/redeemed/revoked/expired. Return expires_at in invitation issuance/list objects. Expired tokens return 404 not_found. Redeeming as S returns 409 invite_used, not a downgrade. Revoking a redeemed invitation also revokes its granted TA membership/sockets. A revoked member cannot regain access by join/replay; a new explicit speaker invitation may restore access. Join tokens last until rotation/revocation/room end. Rotation revokes the prior token but not existing memberships.

End invalidates join/invite tokens and all mutation permissions except the exceptions below, closes the active pointer without altering published copies, and retains read-only access for unexpired, non-revoked existing members. Revocation invalidates the affected membership for both reads and writes immediately. Reject new product mutations after end except access revocations and Bee unbinding; authorized saved-state reads remain available. A new-key repeated end returns existing ended state without another change. A written-Q&A question reference must belong to the room; null captures active Q at commit, if any. Copy its evidence (or [] if absent) without a dependency call. Status markers are private workflow, not replies/moderation.

### Resource shapes

State example (A view):

~~~json
{"room":{"id":"R","title":"Talk","status":"open"},"membership":{"id":"M","role":"audience"},"revision":9,"sequence":9,"active_question":{"id":"Q","candidate_id":"C","text":"Is X clear?","options":["clear","partly_clear","need_help"],"published_at":"T"},"my_response":{"question_id":"Q","selection":"clear","updated_at":"T"},"dashboard":null,"bee":null}
~~~

Room status is open/ended. Revision equals the durable sessions per-room sequence, incremented once per committed room change, never on reads/replay. Active_question/my_response may be null; my_response covers only active Q. S/T active_question uses the full Published shape; S/T receive my_response:null, dashboard {"respondents":2,"counts":{"clear":1,"partly_clear":1,"need_help":0},"percentages":{"clear":50,"partly_clear":50,"need_help":0}}, and Bee projection {"binding_id":"B","status":"connected","has_gaps":false,"pending_command_id":null}. Bee projection additionally has failure:null or the safe binding/delivery failure object below so exhausted commands are visible. At zero respondents percentages are all null; without active Q counts are zero. Bee status: unbound/pending/connected/disconnected/failed; binding_id may be null. It is a possibly lagging projection, never a replay guarantee. No candidate or Q&A lists are embedded in State. S also gets join_link_enabled (boolean), never a stored join URL; omit this field for T/A.

Published is the audience active_question example shape plus evidence_refs and generation below for S/T only. A receives only displayed fields; archived list items additionally include own my_response or null. Published copies never change when closed/replaced/ended.

Candidate:

~~~json
{"id":"C","room_id":"R","job_id":"J","text":"Is X clear?","evidence_refs":[{"event_id":"E","binding_id":"B","conversation_uuid":"external-value","arrival_index":12}],"generation":{"model_id":"stub-v1","prompt_version":"understanding-v1","settings":{},"generated_at":"T"}}
~~~

Candidates are immutable. Sessions fetches C from topics, verifies room, and rechecks role/room status inside publication transaction; copy text/evidence/generation verbatim. A new key may deliberately republish C as a new Q. No editing candidate content in MVP.

Job: {"id":"J","room_id":"R","status":"queued","revision":1,"candidate_ids":[],"failure":null}. Status queued/running/succeeded/failed; terminal states never regress. Failure is null or {"code":"insufficient_evidence","message":"No usable context"}; other failure codes model_unavailable/invalid_output. Job revision is topics-owned, distinct from room revision; delivery failures are separate coordination metadata below. A new generation request creates a new J.

Written: {"id":"uuid","room_id":"R","author_membership_id":"M","text":"Please explain X","published_question_id":"Q","evidence_refs":[],"status":"open","created_at":"T","updated_at":"T","revision":9}. Its revision is the last sessions change affecting it; question ID can be null. A receives only own records with evidence_refs omitted; S/T receive full records. No audience route reveals another participant's responses or Q&A.

### WebSocket synchronization

Server-to-client JSON only; mutations stay HTTP. Change envelope:

~~~json
{"event_id":"E","event_type":"room.changed","schema_version":1,"room_id":"R","occurred_at":"T","sequence":9,"revision":9,"payload":{"resources":["state"]}}
~~~

Chosen semantics are invalidation, not domain patches. Resources are state/questions/written_questions/generations/candidates/invitations; clients refetch authorized resources. Do not overwrite a newer State with an older HTTP response. List refreshes are serialized/coalesced; an invalidation during a fetch requires another fetch after it completes. Response changes invalidate state for author and S/T; Q&A invalidates written_questions only for author and S/T; invitations/access details are S-only; job/candidate notifications S/T-only. Publication/end invalidates state and questions for everyone. For invisible sequences send room.cursor with {} payload and the same sequence/revision, not private IDs/text (sequence metadata may reveal activity, not content).

Initial connection or unusable cursor sends room.snapshot with payload {"state":State}, sequence/revision at that atomic snapshot, and a fresh frame UUID. Snapshot acquisition must buffer/replay strictly later durable events without a race. For this MVP always send a fresh snapshot on reconnect, even when after is supplied (also for future/pruned cursors). Catch up contiguous sequences after that snapshot, then send room.synced with {} at the last delivered sequence and a fresh frame UUID before continuing live delivery. The cursor is advisory, never authorization; filter under current membership permissions. Snapshot invalidates all permitted cached lists. Snapshot/synced do not increment revision. Ignore duplicate change sequences; a gap forces reconnect without after. Bound queues to 256 frames, closing overflow with 1013; reconnect with capped exponential backoff 1–30 seconds and resync. A 1008 requires access recovery, not infinite retries. End sends a normal change and permits read-only sockets.

### Internal HTTP and delivery

Internal paths are service-local, never exposed by the browser proxy. Every request requires `Authorization: Bearer <service credential>`, per-caller credentials and explicit receiver allowlists. Missing/invalid credential is 401; wrong caller is 403. Bind envelope producer to authenticated caller. Secrets stay in private environment/config and are redacted; use TLS outside an isolated local Compose network. Browser cookies never authenticate services. Sessions authorizes user actions; services access only their own databases, including inbox/outbox/read projections.

Shared command/event envelope (revision belongs to the named producer aggregate, not necessarily the sessions room):

~~~json
{"event_id":"E","event_type":"transcript.received","schema_version":1,"producer":"bee-connection","room_id":"R","aggregate_id":"B","revision":12,"occurred_at":"T","payload":{"binding_id":"B","conversation_uuid":"external-value","received_at":"T","arrival_index":12,"raw_payload":{"conversation_uuid":"external-value","utterance":{"text":"X means ...","speaker":"sample"}}}}
~~~

Raw payload illustrates only documented sample fields, not a new Bee schema. Preserve received payload unchanged; event UUID, received_at and arrival_index are locally assigned. No source timestamp, stable utterance ID, speech ordering or replay cursor is implied. Repeated text still gets a new UUID. Unparseable payloads remain raw Bee records, without fabricated transcript text. Transcript revision equals per-binding arrival_index; lifecycle/gap projections have separate monotonic aggregate revisions (stream identity = producer + event_type + aggregate_id). Insert every unseen transcript event even if its index is lower than previously received; context ordering is local arrival order, not speech order.

| Caller → receiver | Method/path | Request | Response/effect |
|---|---|---|---|
| sessions → Bee | `POST /internal/v1/commands` | envelope bee.binding.set, aggregate R, revision = command's sessions room revision, payload {"conversation_uuid":"external-value"}; bee.binding.remove uses {} | 202 {"event_id":"E","status":"accepted"}; durable inbox; Bee allocates B and reports result |
| Bee → topics | `POST /internal/v1/events` | transcript.received above; or bee.gap.recorded with payload {"binding_id":"B","disconnected_at":"T","reconnected_at":null,"recoverable":false} | 200 {"event_id":"E","status":"applied"}; inbox + context commit |
| Bee → sessions | `POST /internal/v1/events` | bee.binding.updated, aggregate R, Bee projection revision; payload {"command_id":"E","binding_id":"B","status":"connected","has_gaps":false,"failure":null} | 200 applied receipt; update Bee projection and invalidate state |
| sessions → topics | `POST /internal/v1/commands` | generation.requested, aggregate J, revision 1; payload {"job_id":"J","requested_by_membership_id":"M"} | 202 accepted receipt; unique queued J + inbox |
| sessions → topics | `GET /internal/v1/rooms/{id}/generations` | same pagination as browser | 200 {"items":[Job],"next_cursor":null} |
| sessions → topics | `GET /internal/v1/rooms/{id}/candidates` | same pagination as browser | 200 {"items":[Candidate],"next_cursor":null} |
| sessions → topics | `GET /internal/v1/rooms/{id}/candidates/{candidate_id}` | — | 200 Candidate; 404 if not in room |
| topics → sessions | `POST /internal/v1/events` | generation.updated, aggregate J, revision = Job.revision; payload {"job":Job} | 200 applied receipt; update coordination projection, invalidate generations/candidates, never publish |

Bee binding status values match State; binding_id is null when unbound/failed before allocation, command_id is null for unsolicited connection changes. Binding failure is null or {"code":"binding_conflict","message":"Conversation already bound"}. One conversation cannot be actively bound to multiple rooms. Emit gap records on disconnect and reconnect with the same gap aggregate UUID and increasing revision; reconnect timestamp becomes non-null, recoverable stays false. Emit Bee projection updates so S/T see has_gaps and connectivity without querying raw events.

Commands acknowledged 202 are durable, not necessarily executed. Duplicate command/event delivery returns 200 {"event_id":"E","status":"duplicate"}, without repeating effects. Dedup by producer + event_id and envelope fingerprint; same ID/different envelope is 409 idempotency_conflict. Commit inbox, effects and any outgoing event together. Unsupported type/version is 400 invalid_request; other errors use the standard error envelope. Retry network errors, 429 (honor Retry-After) and 5xx with capped backoff and maximum five automatic attempts; do not retry permanent 4xx automatically. Persist exhausted failures for inspection/explicit retry using the same event_id; no eventual-delivery promise.

Sessions chooses J and saves the generation command/outbox before browser 202. Topics owns all actual job states and candidates. Sessions keeps only coordination/read projections for accepted requests, exposes queued pending requests at provisional Job revision 0, and uses generation.updated to notify browsers. Candidate reads proxy topics; generation lists are served from sessions' accepted-command/Job notification projection and may lag (sessions-owned pagination), with each item shaped {"job":Job,"delivery":{"status":"pending","failure":null}} (status pending/delivered/exhausted; failure null or {"code":"delivery_failed","message":"Command delivery exhausted"}). Delivery failure does not turn an unknown remote job into a terminal failed Job; operator retry may still recover it. Topics Job objects themselves have no delivery field. After topics acceptance, notifications carry queued/running/succeeded/failed and candidate_ids. Failed event deliveries remain visible to operators, not silently treated as synchronized.

For projections, acknowledge unseen stale event IDs but ignore lower/equal aggregate revisions; additive transcripts are the exception above. Bee applies set/remove commands in one shared per-room command revision stream (regardless of event_type) using sessions revision so a delayed set cannot undo a newer remove. Bindings never retroactively reassign raw events. End queues Bee removal; already captured/late context and generation results may persist but cannot reopen/publish into ended rooms. No cross-database reads or cross-service distributed transaction is claimed.

### Model adapter boundary

Chosen application-level interface, not a provider API: input {"job_id":"J","passages":[{"evidence_ref":{"event_id":"E","binding_id":"B","conversation_uuid":"external-value","arrival_index":12},"text":"X means ..."}],"prompt_version":"understanding-v1","settings":{}}; output {"text":"Is X clear?","evidence_refs":[EvidenceRef],"model_id":"configured-exact-id"} or typed Job failure. EvidenceRef has exactly the reference shape in Candidate. Topics selects recent plus relevant earlier saved context, validates nonempty text and references against supplied passages, and attaches generated_at. Models cannot invent evidence IDs or publish. One successful job produces one immutable candidate in MVP.

Configure adapter kind, exact model ID, prompt version/settings and bounded request timeout server-side. Provider URL/auth/schema belong to a separately evaluated adapter, not this contract. Default deterministic stub uses labeled synthetic fixtures, returns fixed evidenced text, fails empty evidence as insufficient_evidence, and supports injected timeout/invalid-output tests. Model ID stub-v1 never demonstrates actual Bee ingestion, model selection or production readiness; no paid provider call is implied.

## Bee boundary and model evidence

The researched local proxy exposes unauthenticated SSE `GET /v1/stream` on `127.0.0.1:8787`. It is **local-only and must never be exposed publicly**, proxied through Caddy, or sent directly to browsers. The adapter must use an explicit private local connection; do not change the proxy to a public bind to solve container reachability. See [Bee realtime documentation](https://docs.bee.computer/docs/realtime).

Chosen local ingestion topology: run bee-connection on the same host/network namespace as the local Bee CLI proxy and have its server-side SSE client connect directly to http://127.0.0.1:8787/v1/stream. The CLI does not forward through sessions or expose a webhook; no additional ingress route exists in this MVP. If other services run in Compose, expose only the authenticated Bee internal HTTP service on a private host/container route; keep the unauthenticated proxy bound to host loopback. Do not use a public bind or Caddy route to solve reachability.

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
