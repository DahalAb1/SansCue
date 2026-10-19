# Live audience feedback and moderated questions: demonstration plan

Status: implemented on this branch except live Bee device and live model
verification; see [Implementation status](#implementation-status) at the end.

Suggested branch: `feat/live-audience-feedback-and-moderated-questions`

## Objective and scope

Make SansCue ready for a controlled demonstration: the speaker talks, Bee supplies
transcript context, AI drafts useful check-in questions, a moderator publishes
them, the audience responds, and the speaker sees a live response chart. Audience
members can also ask private written questions and receive moderator replies.

The existing `ta` role means Teaching Assistant. In this document, **moderator**
describes that person's job. Keep the existing role identifier unless a separate
change requires renaming it; a database-wide role rename is outside this scope.

This plan narrows the next implementation slice of [mvp-plan.md](mvp-plan.md).
Use its service boundaries and existing access/reliability behavior, but follow
the role and presentation decisions below where the older plan differs. Complete
the demo work before discussing deployment. The user requested this handoff plan;
broader documentation and main README updates remain deferred until deployment.

## Starting point: verify before implementation

The checkout inspected while preparing this plan contains:

- A Preact frontend, three Rust services, and separate PostgreSQL databases.
- Operator credential login, room creation, audience links/QR codes, and moderator
  invitations. A developer provisions the host credential locally; there is no
  self-service speaker signup.
- Speaker-only question publication and version editing. Moderator publication
  and written replies need backend as well as frontend changes.
- Audience feedback counts and percentages displayed as text to staff.
- Private audience questions visible to their author and room staff, without a
  staff reply workflow.
- A deterministic `StubV1Generator`, not an active AI model integration.
- Bee event processing, synthetic replay, room binding, and delivery infrastructure,
  but no live Bee transport in this checkout. Real-device behavior is unverified.

Earlier conversation referred to model and live-Bee code from a different working
state. Inspect the actual branch before editing. Reuse working integrations if
present; otherwise identify and implement the missing pieces. Do not describe
placeholder output or synthetic replay as a live AI/Bee result.

Useful starting files:

- `web/src/app/Room.tsx`, `App.tsx`, `api.ts`, `navigation.ts`, and `live.ts`
- `web/src/styles.css`
- `services/sessions/src/access.rs` and Sessions migrations/tests
- `services/topics-and-questions/src/lib.rs` and `src/main.rs`
- `services/bee-connection/src/` and its README/contracts
- [Question generation design](question-generation-design.md)
- [Bee verification procedure](bee-device-verification.md)
- [Synthetic acceptance procedure](../services/acceptance/README.md)

## Agreed product behavior

| Participant | Responsibilities and interface |
| --- | --- |
| Speaker | Creates/manages the demo room, connects the recording, invites the moderator, shares the audience link, and views the current published question with a live response chart. No publishing controls. |
| Moderator (`ta`) | Reviews AI drafts and their evidence; edits, dismisses, and publishes questions; replies to individual audience questions. |
| Audience | Joins by link/QR code, responds to the published question, asks private written questions, and reads replies. No Bee account required. |
| AI | Automatically drafts questions from the talk. Does not publish them or automatically answer audience questions. |

Preserve the audience and moderator layouts wherever practical. The requested
chart is specifically for the speaker view, not a redesign of every role.

For the controlled demo, arrange for a moderator to be present. If none is
present, drafts remain unpublished. A future product policy for sessions without
a moderator is still undecided; do not introduce automatic publishing as a fallback.

## Implementation order

### 1. Speaker feedback chart

- Replace the speaker's text-only feedback summary with an accessible horizontal
  bar chart for **Clear**, **Partly clear**, and **Need help**.
- Show the current published question, response count, and counts/percentages for
  each category. Percentages refer to respondents, not total attendance.
- Update from the existing live room state. Use actual saved responses; no invented
  trend lines, participation rates, or sample data in the real dashboard.
- Handle no published question, zero responses, reconnecting, and ended-room states.
- Include readable labels and values; meaning must not depend on color alone.
  Check small screens and keyboard access. Prefer existing frontend capabilities
  over adding a large chart dependency for three bars.
- Remove speaker publish/edit-publication controls as part of the permission change
  in phase 2. Keep the current published question prominent while presenting.

Acceptance: submitting or changing an audience response updates the correct
speaker chart without a page refresh. Zero responses never imply everyone is clear.
The moderator and audience interfaces retain their existing overall layout.

### 2. Moderator publication and private replies

- Move publication authority to the room's active moderator role. Enforce this in
  both normal backend requests and idempotent request replay; hiding a button alone
  is insufficient. Speakers and audience members cannot publish through direct API calls.
- Let moderators inspect source evidence, edit a draft before publishing, dismiss
  an unsuitable draft, and publish a selected draft. Persist edits and dismissals
  so refresh/reconnect does not undo them.
- Keep one active published question per room. Publishing a new question replaces
  the active question; a background model result never changes it automatically.
- Preserve immutable publication history and source evidence. If retaining the
  existing published-version editor, make it moderator-only and keep response
  attribution explicit so changed wording is not silently presented as new feedback.
- Add a reply form to each audience question and display replies in its thread,
  identified as coming from the moderator. Preserve unsent reply text across tab
  changes using the existing draft conventions.
- Replies are visible only to the original audience author and authorized room
  staff. Other audience members and other rooms cannot read them.
- Require active membership and an open room for mutations. Preserve CSRF checks,
  request idempotency, and room isolation. Do not broaden moderator access to
  speaker-only room management or operator actions.
- Refresh threads through the existing live update mechanism. Avoid broadcasting
  private question/reply bodies to everyone in a room.

Acceptance: a moderator can edit, dismiss, publish, and reply; the question author
receives the reply without a refresh. Retrying a successful action does not create
duplicates. Unauthorized, revoked, cross-room, and ended-room writes are rejected.

### 3. Context-aware automatic question drafting

- Verify the current generator first. If it is still the placeholder, integrate a
  real model through the existing generator interface. Confirm available provider
  access; do not assume any named provider or credential is configured.
- Generate short comprehension check-ins that fit the existing Clear / Partly clear
  / Need help responses. Do not silently turn them into factual quizzes that require
  a different answer interface.
- Build a bounded, coherent window of recent speech. Require enough context to
  identify an explained idea; skip greetings, filler, unfinished explanations,
  ambiguous speech, and stale transcript backlog.
- Add a configurable cooldown and recent-question history to reduce repeated ideas
  and rephrased duplicates. Gates should avoid unnecessary model calls where possible.
- Retain exact transcript evidence with each draft and validate model output.
  Treat spoken instructions as transcript data, not instructions to the generator.
  Abstaining is preferable to an unsupported question.
- Keep a small, fresh review queue, with durable dismissal and a deliberate expiry
  policy. Expired or dismissed drafts must not become publishable through a stale UI.
- Preserve job retries, bounded timeouts, duplicate protection, and service-owned
  storage. A slow or unavailable model must not block replies or audience feedback.

Proposed tuning starting points, **not finalized product requirements**: at least
60 words of context, a 90-second minimum interval between drafts, at most three
visible recent drafts, and a 10-minute draft expiry. Test these against realistic
speech before settling on values; avoid a policy that produces either a flood or
no usable draft during the demonstration.

Acceptance: test substantive speech, filler, repeated explanations, insufficient
context, transcript gaps, and provider failure. Drafts are relevant, traceable,
and bounded in frequency; none reaches the audience without moderator publication.
Report live model quality separately from deterministic/mock test results.

### 4. Controlled demonstration setup and Bee connection

- Use existing operator access for this demo. Provision the presenter privately
  before rehearsal; never embed the credential in frontend code or demo fixtures.
- Make the visible setup sequence clear: create room, invite moderator, share the
  audience QR code, connect the presenter's recording, and verify incoming speech.
- Reuse the existing authenticated Bee setup if available. If the current branch
  lacks the live adapter, implement the smallest supported transport into the
  existing event/binding pipeline. Use current official Bee documentation and
  verify with the device; do not assume a browser OAuth flow exists.
- Keep developer credential/transport preparation outside the audience-facing
  flow. This is a controlled presenter setup, not public multi-user onboarding.
- Associate the intended recording with exactly the intended room. Prefer a simple
  recording selection/confirmation over asking the presenter to copy internal IDs.
  Never route an arbitrary personal conversation merely because it arrives next.
- Distinguish connected, waiting for speech, receiving transcript, reconnecting,
  and disconnected states. A successful login does not prove transcript delivery.
- Include a short spoken connection check before presenting. Reconnect to the same
  recording, avoid duplicate events, and withhold new drafts when context is stale
  or incomplete. Existing published questions and replies should remain usable.
- Ending the room stops further ingestion for that room and new participation.
  Do not imply that it remotely stops Bee's microphone or deletes recorded data.

Acceptance: the intended live recording reaches only its room, generates a draft,
and recovers from a connection interruption without duplicate publication or replies.
If real Bee access is unavailable, report that limitation. Synthetic replay can
verify plumbing but does not complete the live demonstration requirement.

### 5. Rehearse and verify the complete flow

Use separate browser sessions for speaker, moderator, and audience. Include a
second audience member to verify privacy, and a second room for isolation checks.

1. Create the room with existing host access and invite the moderator.
2. Join as audience through the generated link/QR code.
3. Connect Bee and verify that a short spoken phrase arrives in the correct room.
4. Explain a substantive idea and observe an automatically generated private draft.
5. Review its evidence, edit if necessary, and publish as moderator.
6. Submit different audience ratings and verify the speaker chart and total.
7. Ask a private audience question, reply as moderator, and verify only its author
   and staff can see the conversation.
8. Dismiss a draft; confirm it stays dismissed after refresh. Repeat an explanation
   and verify question generation does not flood the review queue.
9. Interrupt and restore a connection; verify state recovery and duplicate handling.
10. End the room and verify further participation/publication is rejected.

Run existing frontend tests and build (`npm test --prefix web`,
`npm run build --prefix web`). Run Rust formatting, linting, and relevant service
tests, including database-backed permission, persistence, and retry tests. Consult
each service's test instructions for disposable database requirements. Add focused
regression coverage for the new behavior; do not claim skipped integration tests
passed. Visually inspect the speaker chart and reply flow in desktop and mobile views.

## Deferred until after the demo work

- Hosting selection, production deployment, domains, and production operations.
- Public speaker signup, email-link login, and a persistent “My sessions” page.
- Self-service hosted “Connect Bee” onboarding for multiple independent speakers.
  The desired future flow is sign in, create a session, approve Bee access, invite
  a moderator, and share the audience QR code. Hosted authorization feasibility
  and per-user connection isolation must be validated before promising this flow.
- Automatic publication when no moderator is present, or AI-written audience replies.
- Broad frontend redesigns, new charts for other roles, and additional analytics.
- Main README refresh and broader documentation reconciliation after deployment.

## Handoff expectations

Implement in small, reviewable steps following the order above. Preserve unrelated
working-tree changes and existing service boundaries. Do not deploy or expand into
public account onboarding as part of this slice. At handoff, summarize changed
behavior, checks actually run, how to rehearse the demo, and any remaining live Bee
or model-access blockers. Distinguish completed features from proposed settings
and unverified integrations.

## Implementation status

This branch includes `main`'s live Bee adapter and Claude generator, then adds:

- **Speaker chart.** The speaker's Dashboard shows the published question,
  respondent total, and labelled bars with counts and percentages of
  respondents. It covers no question, zero responses, reconnecting, ended
  rooms, and responses given to an earlier wording. Moderator and audience
  layouts are unchanged apart from moderator controls and reply threads.
- **Moderator authority.** Publish, draft edit, dismiss, version correction and
  Q&A replies require the `ta` role in both fresh requests and idempotent
  replays. Speakers see drafts read-only. Edits and dismissals persist; the
  queue shows at most three drafts, which expire ten minutes after arrival or
  the latest edit. Replies are visible only to the author and staff, and their
  live invalidations target only those members.
- **Generation.** Delivery-level gates, recent-draft duplicate suppression,
  prompt hardening and output validation are described in the
  [design decision](question-generation-design.md), with the scripted-lecture
  evaluation of the proposed thresholds.
- **Bee setup.** Listening-mode adoption is removed: the speaker chooses one of
  the recordings the host worker reports, and other recordings' speech is
  dropped. The setup tab orders the steps (moderator, audience link,
  recording, speech check) and distinguishes stream not running, reconnecting,
  waiting for speech, and receiving transcript. Ending a room unbinds Bee.

Verified locally: Rust format/clippy and all three service suites against
PostgreSQL 16 (Topics with `--test-threads=1`, because its older test files
share one schema), web tests and build, the updated synthetic acceptance, and a
multi-browser walkthrough of the full flow. The walkthrough used a simulated Bee
CLI, so it checks plumbing only.

Not verified: a real Bee device or Bee CLI session, live model output quality,
and Docker/Compose startup. No model credential was available.

### Rehearsal

1. Provision the operator (`python3 deploy/provision_operator.py`), start the
   stack, and run `deploy/run-live-bee-worker.sh` on the presenter's computer
   with Bee authenticated (see `deploy/README.md`). Optionally configure a
   model credential in `deploy/.env`; without one, drafts come from the labelled
   placeholder.
2. Sign in at `/operator`, create the room, and follow **Room access**: create
   the moderator invitation and open it in the moderator's browser; show the
   audience QR code and join from two audience devices.
3. Start recording on the Bee, speak, choose the recording, then say "SansCue
   connection check" until the tab shows **Receiving transcript**.
4. Explain an idea for at least 30 seconds. The moderator reviews the draft and
   its evidence, edits if needed, and publishes. Audience members respond; the
   speaker's Dashboard updates live.
5. An audience member asks a written question; the moderator replies; confirm a
   second audience member cannot see it. Dismiss a draft and reload.
6. Stop and restart the Bee worker; the tab shows "Bee listener not running",
   then recovers. End the room and confirm participation is closed.
