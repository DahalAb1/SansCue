# Bee device / live API verification runbook

**Status: partial live evidence recorded.** [PR #2](https://github.com/DahalAb1/SansCue/pull/2)
reports Apple Watch recording pickup, room transcript delivery and drafts on
local Compose. [PR #3](https://github.com/DahalAb1/SansCue/pull/3) reports live
transcript visibility for speaker and TA. These reports do not fill every
scenario below. Complete the remaining scenario records before full Step 7
acceptance; preserve the reported live passes. Synthetic replay remains separate.

## Preconditions and safety

- Use a dedicated test room and a non-production SansCue deployment/database.
- Have authorized access to the physical Bee source and the actual transport
  (CLI, MCP, or Agent Skill) that will be used for the demo. Record versions;
  do not assume a protocol from the synthetic fixture.
- Confirm permission to capture the selected speech. Use consented test speech
  or a controlled recording; redact participant names and sensitive content in
  notes. Never put API keys, bearer tokens, or unredacted transcripts in this
  repository or in test logs.
- Record the SansCue commit, UTC run time, device/transport version, source
  conversation identifier type, and which API operations were used. Store
  private raw captures outside Git with access limited to the test operator.
- Keep the test within a short, bounded session. Stop if events are arriving in
  the wrong room or authorization behaves unexpectedly.

## Scenarios

For every scenario, record **Pass / Fail / Blocked**, the observed behavior,
sanitized evidence reference, and any deviation. A blocked scenario is not a
pass.

1. **Connect and bind.** Start the real Bee stream/API, bind its source
   conversation to the dedicated SansCue room as the authorized speaker, and
   observe at least several new transcript events. Confirm events are accepted
   only under the active room binding and reach the Bee-owned ingest path.
2. **Identity and order.** Compare source-provided event/conversation IDs,
   sequence values, timestamps, and arrival order. Record scope and reset
   behavior for each source field. Confirm SansCue only deduplicates on source
   identity/sequence semantics demonstrated by this run; arrival ordinal is not
   treated as speech order. Check repeated text with distinct source IDs is not
   silently discarded.
3. **Retry and duplicate delivery.** Cause or simulate a transport/API retry
   using the supported Bee client controls. Confirm a repeated source event has
   one downstream effect while the raw observation/rejection evidence remains
   inspectable. Do not manufacture a Bee protocol request if no supported retry
   control exists; record that subcheck as blocked instead.
4. **Disconnect and recovery.** Interrupt the source connection, continue
   consented test speech, then reconnect using the supported procedure. Record
   whether Bee exposes history/backfill, its cursor/sequence semantics, and the
   recovered range. Verify known gaps are recorded only when supported by
   source evidence. If completeness cannot be established, the system must show
   suspected incompleteness rather than claim a complete transcript.
5. **Binding changes.** Unbind the test conversation and, if supported, bind a
   second test conversation or room. Verify delayed/stale binding commands do
   not associate later events with the previous room. Verify unbound events do
   not create room-visible candidates.
6. **End-to-end delivery.** For accepted events, verify the Bee outbox delivery
   is acknowledged only after Topics accepts it, repeat delivery is
   idempotent, and a resulting candidate remains unpublished until moderator publication and
   staff-only. This checks integration plumbing, not model quality.
7. **Stop and cleanup.** Stop the stream, end the test room, revoke temporary
   access, and remove private test captures according to the agreed retention
   policy. Confirm no credentials were written to logs or Git.

## Run record

Copy this block for each run and keep it with the sanitized evidence:

```text
Date/time UTC:
SansCue commit:
Bee device / source:
Transport and version (CLI / MCP / Agent Skill):
Test room identifier (non-secret):
Consent/test-speech reference:
Source conversation identity field and observed scope:
Source event/sequence fields and observed reset/order semantics:
History/backfill capability and observed limits:
Scenarios 1–7: Pass / Fail / Blocked (attach sanitized evidence refs):
Unexpected events / gaps / duplicate effects:
Cleanup completed:
Operator:
```

## Step 7 acceptance decision

Accept only when connection/authentication, room binding, source identity and
ordering semantics, retry/deduplication, disconnect/recovery, and delivery have
observable evidence and no cross-room leakage. Any unsupported source
capability must be documented as a limitation; synthetic fixture results do
not fill a live-device gap. Record a separate pass/fail decision for each
scenario and link the sanitized run record from the project verification log.
