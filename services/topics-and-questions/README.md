# Topics and questions

This Rust service turns room transcript context into private check-in drafts.
It owns its PostgreSQL database and does not read another service's tables.
For generation policy, defaults, and publication behavior, see
[question generation](../../docs/question-generation-design.md).

## Data flow

The Bee worker sends canonical transcript events to authenticated
`POST /internal/v1/transcript-events` on the private service listener. Topics
stores events idempotently and creates generation jobs. A job assembles recent
speech, applies context and cadence checks, and requests a draft or abstention.
The model's evidence quote must match one contributing utterance. Topics adds
the source identifiers and saves an accepted candidate for delivery to Sessions.

Sessions verifies the room and recording association before accepting a
candidate. Only staff can see drafts; only a moderator can publish. Topics has
no public candidate or publication endpoint.

## Configuration

Supply environment variables to the process; Rust does not load `.env` files.
The [local runtime guide](../../deploy/README.md) describes Compose setup.

| Variable | Purpose |
| --- | --- |
| `TOPICS_DATABASE_URL` | Private URL for this service's database |
| `TOPICS_BEE_SERVICE_TOKEN` | Shared authentication with the Bee worker |
| `SESSIONS_INTERNAL_URL` | Sessions' private listener |
| `SESSIONS_TOPICS_SERVICE_TOKEN` | Authentication for candidate delivery |
| `TOPICS_SESSIONS_SERVICE_TOKEN` | Authentication for staff transcript requests from Sessions |
| `HTTP_HOST`, `HTTP_PORT` | Listener; defaults to `127.0.0.1:3002` |

Keep credentials private and expose this listener only to trusted services.
Cadence overrides are positive integers named in
[main.rs](src/main.rs); Compose exposes the overrides described in its runtime guide.

## Model access

The hosted deployment uses **Amazon Bedrock Nova Micro**. Set
`TOPICS_MODEL=amazon.nova-micro-v1:0` and `AWS_REGION=us-east-1`, with exactly one
Bedrock authentication method:

- `AWS_BEARER_TOKEN_BEDROCK`: an authorized Bedrock API key supplied privately.
- `TOPICS_BEDROCK_CREDENTIALS_FILE`: a private credentials file refreshed by the
  host. The [EC2 runbook](../../deploy/PRODUCTION.md#model-credentials) documents
  the instance role, refresh timer, and container permissions.

Leave other provider credentials blank. Nova's structured response is bounded
to 512 output tokens and passes question-shape and transcript-evidence checks.
An optional direct Anthropic path uses `ANTHROPIC_API_KEY` and `TOPICS_MODEL`;
configure only one provider at a time. With no provider credentials,
`topic-window-stub-v1` is an explicit deterministic placeholder, not AI output.

## Delivery and failure handling

Generation jobs use PostgreSQL row locks, ownership tokens, and expiring leases.
A heartbeat extends active work. The default generation deadline is 30 seconds;
timeouts cancel the call and record a retryable error. Transient failures use
bounded exponential backoff; jobs fail permanently after five claims or invalid
output. Only stable error codes are persisted.

The candidate, its pending delivery, and job completion are committed together
while the worker still owns the lease. A separate delivery loop retries Sessions
callbacks. Receivers verify event identity and payload consistency to avoid
repeated effects. Invalid or stale work receives terminal rejection; a duplicate
of an already accepted callback stays accepted even if the room has since ended.

## Verification

[Tests](tests/) cover generation policy, provider output validation, job leases,
and persistent delivery. The [synthetic acceptance guide](../acceptance/README.md)
checks the three services together without model access. Actual Nova calls have
also passed with synthetic speech; the full live Bee-to-hosted-AI rehearsal
remains unrecorded. See the [verification record](../../docs/environment-verification.md).
