# Verification record

SansCue's interaction loop connects spoken context, audience participation,
and information speakers and moderators can use. This record identifies which
parts have been tested with real devices, real model calls, or synthetic input.
Results apply to the revisions and environments described below. Documentation
edits do not constitute a new application test run.

## What has been demonstrated

| Area | Recorded evidence | Scope |
| --- | --- | --- |
| Actual Bee speech | Sanskriti's Apple Watch runs in PRs #2 and #3 | Local Compose; transcript reached the room and staff views |
| Model generation | Nova integration `7f41b0c`, October 20 | Three real requests using synthetic explanations |
| Hosted feedback loop | October 20 hosted acceptance | Synthetic transcript → real Nova draft → moderator publication → audience rating → presenter counts |
| Access boundaries | Hosted and synthetic acceptance | Audience cannot read drafts; presenter and audience cannot publish; private reply isolation checked in synthetic acceptance |
| HTTPS and restart | October 19–20 hosted checks | Public routing, database outage recovery, same-IP EC2 stop/start |
| Database recovery | October 19–20 hosted checks | Three database dumps restored into temporary databases on the same host |

## Live Apple Watch tests

Sanskriti reported the following in the pull request descriptions:

- [PR #2](https://github.com/DahalAb1/SansCue/pull/2), `cfa8fc1`: 88 Rust tests,
  18 frontend tests, and synthetic acceptance passed. Live Apple Watch speech
  reached the local room and drafts appeared. The report does not identify
  those drafts as model-generated.
- [PR #3](https://github.com/DahalAb1/SansCue/pull/3), `5af0f1d`: 92 Rust tests and
  19 frontend tests passed. The live transcript was visible to presenter and
  moderator. A Bedrock Claude request hit the account's daily token limit;
  that run did not produce a live AI draft.

These runs preceded the explicit recording-selection workflow. Rehearse the
current release using the [device verification guide](bee-device-verification.md).

## Deployment and model checks, October 19–20

A separate SansCue EC2 host was deployed at **https://184.193.80.255**. The
OpenClaw development host was left unchanged. Caddy served public HTTPS;
application services and databases remained private. Hosted checks covered
room creation, browser ownership, secure cookies, audience permissions,
unknown-route handling, and request limiting despite forged client-IP headers.

The deployment branch passed 109 Rust workspace tests against disposable
PostgreSQL 17 databases, synthetic acceptance, 25 frontend tests, and the
frontend build. Two generation-reliability tests shared a queue in a parallel
run; the recorded successful workspace run used `--test-threads=1`.

Production image builds and three build-context isolation fixtures passed.
During a Sessions database outage, public liveness stayed 200 and readiness
returned 503, then recovered without restarting Sessions. EC2 stop/start
retained the IP and restarted the six core containers. All three database
dumps restored into temporary check databases on that host; data-only hashes
matched. This did not test recovery onto another machine.

Nova Micro (`amazon.nova-micro-v1:0`, `us-east-1`) became the configured hosted
model. Adapter `7f41b0c` and credential helper `1c9b971` were checked, with 19
library/provider regression tests and 31 deployment checks passing. Three real
requests produced clarity questions with evidence from synthetic explanations
of learning rate, photosynthesis, and hash-table chaining.

Hosted acceptance then passed explicit recording binding, synthetic transcript
ingress, real AI draft delivery, staff-only draft visibility, rejection of
presenter/audience publication, moderator publication, audience rating, and
presenter counts. Transcript submission through rating completion took 4.26
seconds in that single run; it is not a latency benchmark. The test room was
ended and unbound afterward.

The instance role was limited to Nova invocation. Credential refresh and
container file access were checked without copying personal AWS keys to EC2.
Start/stop schedules were configured and read back; their future scheduled
execution was not observed in that record.

## Earlier integration evidence

At `411fba0`, synthetic acceptance covered binding and outbox delivery,
duplicate handling, draft privacy, moderator edits/publication, audience
responses, private replies, counts, and room shutdown. It used a deterministic
placeholder generator. This established service integration, not live device
behavior or model quality.

The original detailed test chronology and preparation checklists are retained
in the team's private archive. Current reproduction instructions are in the
[synthetic acceptance guide](../services/acceptance/README.md),
[local runtime guide](../deploy/README.md), and
[production runbook](../deploy/PRODUCTION.md).

## Checks still needed

- Rehearse the current release from actual Bee/Watch speech through hosted AI,
  moderator publication, and audience feedback.
- Complete device disconnect, recovery, and recording-change scenarios.
- Measure representative question quality, response delay, and expected audience
  load. The 64-WebSocket implementation limit is not a tested room capacity.
- Rehearse off-instance restore and application rollback.
- Confirm judge access and deployment availability for the evaluation period.

These are separate checks. A passed synthetic run does not fill a live-device
or deployment gap. Clarity responses are participants' own reports, not a
validated measure of understanding or evidence of improved communication.
