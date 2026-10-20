# EC2 deployment preparation

Status: the separate SansCue EC2 deployment serves **https://184.193.80.255**.
Public HTTPS, access checks, database outage recovery and automatic EC2 stop/start
recovery passed. Bedrock Nova Micro is active; the hosted synthetic-transcript
flow passed with a real generated draft, moderator publication and audience rating.
Live Bee transport, audience load and broader disaster recovery remain open.
Exact evidence is in [environment verification](../docs/environment-verification.md).

## Deployment layout

One Linux EC2 host runs Caddy/static web, Sessions, Topics and three independent
PostgreSQL 17 databases. Only TCP 80/443 are public. Sessions' private listener
(3401), Topics (3402) and the Bee database (55433) remain bound to EC2 loopback.
The trusted Mac runs the authenticated Bee worker through SSH local forwards.
The container worker is disabled by default to prevent two workers racing.
This is a single-host MVP, with no high-availability or measured capacity claim.
Sessions currently caps live browser connections at 64 per process; establish
the rehearsal audience size before opening registration.

## Infrastructure inputs

- An AWS account/region, Linux EC2 host with Docker Engine and Compose **2.24.4
  or later**, Python 3, Git, persistent encrypted EBS storage and sufficient
  disk/RAM for the Rust image builds. Instance sizing remains to be measured.
- A public IPv4 address for the initial demo. A custom domain is optional later;
  when using one, point its DNS at the instance. Do
  not publish an AAAA record unless IPv6 reaches the instance.
- Security group: public inbound TCP 80 and 443; SSH TCP 22 from the trusted
  administrator/Mac IP only. No inbound database or service ports. Verify the
  host firewall as well. Keep SSH keys private and verify the server host key.
- A separate checkout/host for staging. Named production volumes are distinct
  from local development; do not reuse existing demo database volumes.

Caddy manages certificates and HTTP-to-HTTPS redirects; `/data` is persisted
across container recreation. See [Caddy HTTPS requirements](https://caddyserver.com/docs/automatic-https).
Public-IP mode uses explicit short-lived ACME certificates (see below). The
production override uses Docker's [replacement/reset semantics](https://docs.docker.com/reference/compose-file/merge/)
to remove the local web/DB port configuration.

## Configure on EC2

Use a release revision reviewed by the team; record `git rev-parse HEAD`. Work
in a private checkout whose access is limited to trusted deployment operators.

```sh
umask 077
cp deploy/.env.production.example deploy/.env
chmod 600 deploy/.env
```

Edit the private file locally on EC2: set `SANSCUE_DOMAIN` to the public IPv4
address (or a bare lowercase hostname later), and fill all database passwords,
bootstrap passwords and service tokens with distinct random values from a
password manager (32 random bytes encoded as hex works). Use plain `KEY=value`
syntax without quotes, spaces, comments after values or interpolation. Do not
export competing configuration variables in the shell.

```sh
python3 deploy/provision_operator.py
python3 -B deploy/check-production.py
sh deploy/production.sh config --quiet
sh deploy/production.sh build
sh deploy/production.sh run --rm --no-deps web caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
sh deploy/production.sh up -d --wait --wait-timeout 180
sh deploy/production.sh ps
```

`provision_operator.py` writes the operator hash, epoch and independent HMAC
key into this file, and the plaintext credential into the private
`local-data/operator/credential` file. Read it privately for the current operator
login; rerunning rotates access. The tool's printed generic local restart hint
must be replaced with `sh deploy/production.sh up -d --wait sessions` on EC2.
The hackathon uses open demo room creation instead of email login. The host
credential remains private administration access.

Always use the production wrapper for this deployment. It selects both Compose
files and the project, checks private settings without printing values, and
preserves the production volume names. Never publish resolved Compose config
or container environments: they contain credentials. Stop with
`sh deploy/production.sh stop`; ordinary `down` retains data, but **never use
`down -v` on this deployment**.

On fresh volumes, PostgreSQL initializes separate bootstrap and application
roles. Services get only their own database owner's credential, without
superuser, database-creation or role-creation privileges. The owner can apply
its service migrations. Initializer scripts and changed env passwords do not
modify existing volumes; rotate existing DB passwords through an explicit
database operation before changing service configuration. PostgreSQL's
[image initialization behavior](https://hub.docker.com/_/postgres) applies.

## Model credentials

For Claude Haiku through Cloudflare, use `CLOUDFLARE_API_TOKEN` and
`CLOUDFLARE_ACCOUNT_ID`, with `TOPICS_MODEL=anthropic/claude-haiku-5.5` in the
private runtime file. Leave Anthropic and Bedrock credentials blank. The token
must have Workers AI Read access to that account. EC2 calls Cloudflare's
`/accounts/{account_id}/ai/v1/messages` endpoint using bearer authentication;
no Worker or hosting migration is required. Gateway request logging and caching
are bypassed for transcript requests. Structured responses still pass the same
grounding checks and moderator review. Third-party Haiku inference uses AI Gateway
credits; the student Workers subscription waiver does not establish that these
credits are available. Verify a real request before changing the hosted service.
See [Cloudflare REST authentication and billing](https://developers.cloudflare.com/ai-gateway/usage/rest-api/).

Configure at most one provider in EC2's private `deploy/.env`. All blank runs
the clearly labeled placeholder; a transcript reaching the UI does not prove
AI generation. The hackathon's selected provider is **Amazon Bedrock Nova Micro**
(`TOPICS_MODEL=amazon.nova-micro-v1:0`, `AWS_REGION=us-east-1`). Set
`TOPICS_BEDROCK_CREDENTIALS_FILE=/run/bedrock/credentials.json` and leave other
provider credentials blank. Nova uses its native request/response format, with
a 512-token output bound and the same transcript grounding, cadence, and moderator
review. A hosted page without incoming speech does not itself make model calls.

Only the SansCue EC2 instance gets the `sanscue-demo-nova-micro` instance role.
Its permission is `bedrock:InvokeModel` on
`arn:aws:bedrock:us-east-1::foundation-model/amazon.nova-micro-v1:0` only.
`refresh-ec2-bedrock-credentials.py` reads that role through host IMDSv2 and
atomically refreshes the private credential file. The included systemd service
and timer run every three minutes. Install their copies in `/etc/systemd/system/`,
run `systemctl daemon-reload`, `systemctl enable --now sanscue-bedrock-credentials.timer`,
and `systemctl start sanscue-bedrock-credentials.service` before activating Topics.
They use `/opt/sanscue` as the checkout path. No personal AWS keys are copied to EC2.
Install `sanscue-bedrock-startup.conf` as
`/etc/systemd/system/sanscue.service.d/bedrock.conf` so startup attempts a
credential refresh before launching the application; a refresh failure does not
prevent the core website from starting.

The mount is read-only in Topics. Credential directory/file modes are 700/600
and owner UID 10003; the container's primary GID is not assumed to equal its UID.
The generator re-reads credentials per request, so refresh needs no container
restart. IMDSv2 remains required with hop limit 1. The timer also refreshes
after 15 seconds at boot; credential errors are retryable. Run a real request to verify
IAM access, quota, generation, and cost before the demo. The user's reported
credit balance is not programmatic verification of Bedrock credit eligibility.

## Connect the trusted Mac Bee worker

Keep Bee login on the Mac. Configure an SSH host alias `sanscue-ec2` for the
instance/user/key, then run this in a dedicated terminal on the Mac:

```sh
ssh -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 -o ServerAliveCountMax=3 \
  -L 127.0.0.1:3401:127.0.0.1:3401 \
  -L 127.0.0.1:3402:127.0.0.1:3402 \
  -L 127.0.0.1:55433:127.0.0.1:55433 sanscue-ec2
```

These Mac ports must be free; stop a conflicting local demo first. The tunnel
encrypts service and DB traffic without making their EC2 ports public. The Mac
worker is trusted with the Bee database and two service credentials only.

In the same release checkout on the Mac:

```sh
umask 077
mkdir -p local-data
cp deploy/.env.bee-remote.example local-data/bee-remote.env
chmod 600 local-data/bee-remote.env
# Privately edit the five settings to match EC2; this is trusted shell input.
bee status
cargo build --locked --package bee-connection
SANSCUE_WORKER_ENV_FILE="$PWD/local-data/bee-remote.env" sh deploy/run-live-bee-worker.sh
```

Run exactly one Bee worker. Do not enable the `container-worker` profile while
the Mac worker is running. After an SSH interruption, restore the tunnel and
verify actual transcript delivery again. The Mac must stay awake and connected
throughout the session; stream-running status alone is not proof of delivery.

## Hosted acceptance

From an external browser/network, confirm HTTP redirects to HTTPS and TLS has
no warnings. Confirm `/`, `/api/sessions/healthz` and `/api/sessions/readyz`
return 200, unknown API paths return 404, and direct room URLs load the SPA.
Confirm external access to 3000, 3001, 3002, 3401, 3402, 5432 and 55433 fails.

Use speaker, moderator and audience browsers (including a phone): create a
room, scan the QR, select the Bee recording explicitly, speak a consented test
phrase, observe the staff-only transcript and a draft, publish as moderator,
submit a rating/private question, and verify the chart and private reply.
Test a second room, revoked invitations, ended rooms and WebSocket reconnects.
Record UTC time, release commit, counts/status and failures without tokens or
transcript contents. Check browser cookies are Secure and HttpOnly.

During a staging maintenance window, stop only `sessions-db`, check liveness
200/readiness 503, restart it and confirm readiness recovers without restarting
Sessions. Run the persistence and backup/restore rehearsal below. Keep a
separate gate for expected audience load and full-flow timings.

## Backup, restore and rollback rehearsal

Backups contain private participant data. Use access-restricted encrypted
storage and an off-instance copy; set retention and deletion before real use.
The three databases need separate dumps. For a coordinated maintenance backup,
stop the Mac worker first, then stop web, Sessions and Topics:

```sh
# Run this backup block in a dedicated shell; stop immediately on any failure.
set -eu
sh deploy/production.sh stop web sessions topics-and-questions
umask 077
mkdir -p local-data/backups
chmod 700 local-data/backups
# Use a new private backup directory for each run; do not overwrite a prior dump.
backup_dir=$(mktemp -d local-data/backups/backup-XXXXXXXX)
for service in sessions-db bee-db topics-db; do
  sh deploy/production.sh exec -T "$service" sh -c \
    'pg_dump -U "$SANSCUE_APP_USER" -d "$SANSCUE_APP_DB" --format=custom' > "$backup_dir/$service.dump"
done
```

Verify every dump command succeeded and all three files are present. Validate
each catalog with `pg_restore --list`; a catalog listing alone is not a restore
test. Restore into a **separate staging stack with empty volumes**, bootstrap
roles/databases initialized, and application writers stopped. For each database:

```sh
# On the isolated restore stack, choose the matching service and private dump.
sh deploy/production.sh exec -T sessions-db sh -c \
  'pg_restore -U "$SANSCUE_APP_USER" -d "$SANSCUE_APP_DB" --no-owner --no-privileges --single-transaction --exit-on-error' \
  < /PRIVATE/BACKUP/sessions-db.dump
```

Repeat for Bee and Topics. Start the matching application revision and compare
room state/counts, migrations and binding behavior. Restart with
`sh deploy/production.sh up -d --wait --wait-timeout 180`, then restore the Mac
worker only after checks pass. Verify ordinary `down`/`up` retains state and
Caddy certificates on staging. Never run the local destructive reset recipe.

Before each update, record the release commit, take verified backups and retain
the previous built images. Check out/build a reviewed revision during a planned
window; record readiness and a smoke-test result after startup. Old binaries
may not support new migrations: application rollback is permitted only after
schema compatibility is checked, otherwise recover the matching backups into
a separate stack. The first restore and rollback rehearsal remain release gates.

## Open hackathon access and abuse limits

`SESSIONS_DEMO_OPEN_ROOMS=true` lets any visitor create a room immediately at
`/operator`, without email or approval. The browser session owns the room;
other visitors cannot take its speaker controls by changing a URL. Audience
links still grant only audience access. Keep the creating browser's cookies
and save the room URL; this demo identity expires with the existing browser
session (24-hour absolute / 12-hour idle limits). Host administration can
restore access. Set the flag to `false` and recreate Sessions to disable new
anonymous rooms and existing anonymous speaker authority. Default application
and local configuration are closed; the production example explicitly enables
this hackathon behavior.

Production enables in-process token buckets with a bounded table of 4,096
client IPs. No paid rate-limit service is required. Per-IP limits refill at
600 reads/minute (burst 100), 120 writes/minute (burst 30), 60 bootstraps/minute
(burst 15), and 10 room creations/hour (burst 3). Global limits refill at 3,000
requests/minute (burst 300), 300 bootstraps/minute (burst 100), and 50 room
creations/hour (burst 10). Limits reset on a service restart. Excess requests
receive 429 with Retry-After; bodies are limited to 16 KiB. Existing login,
CSRF, session, role and 64-WebSocket limits remain in effect. These are bounded
MVP protections, not a claim of DDoS resistance or measured 500-user capacity.

Caddy overwrites `X-Sanscue-Client-IP`; Sessions trusts it only under the
production proxy configuration. Do not expose the backend listener directly
or place an unconfigured CDN/proxy in front of Caddy. Conference attendees may
share an IP, so rehearse the expected simultaneous audience before changing
limits. Free room creation does not expose model credentials or Bee's private
service ports. Email login was set aside for this demo; no emails are sent.

## Public IP HTTPS and judging window

The first deployment uses a public IPv4 URL. Caddy explicitly requests the
Let's Encrypt `shortlived` ACME profile and supplies default SNI for clients
opening an IP URL. Use Caddy 2.10.1 or later. These public certificates last
about six days and Caddy renews them automatically; keep TCP 80/443 reachable
and the certificate volume persistent. See [Let's Encrypt IP certificate availability](https://letsencrypt.org/2026/01/15/6day-and-ip-general-availability)
and [Caddy IP configuration](https://github.com/caddyserver/caddy/issues/7118).
This deployment reserves Elastic IP `184.193.80.255`, which remained unchanged in
the executed stop/start rehearsal. A reserved IPv4 address keeps accruing charges
while the instance is stopped (about $0.12/day at $0.005/hour). An automatically
assigned IP would change after stop/start; the initializer reads the public IP
on each boot. Adapt that initializer before introducing a custom domain.

Judging availability requested: **November 9–20, 2026, America/Chicago**.
Keep the instance on for that whole window; stop outside demos and judging to
save compute cost. A t3.small in us-east-1 currently costs $0.0208/hour, plus
$0.005/hour for public IPv4 and approximately $2.40/month for 30 GiB gp3. Twelve
continuous days are about $8.40 before tax, model usage, backups and setup.
The existing t3.large OpenClaw server is a separate budget item; no capacity or
cost reduction on it is assumed. Instance sizing remains subject to rehearsal.

## Checks for configuration changes

```sh
python3 -B -m unittest discover -s deploy -p 'test_*.py'
python3 -B deploy/check-operator-config.py
sh -n deploy/production.sh deploy/init-app-db.sh deploy/run-live-bee-worker.sh
```

The Compose merge tests require an actual standalone Compose v2 binary, without
a Docker daemon. Set `SANSCUE_COMPOSE_BIN=/absolute/path/to/docker-compose`
when running the unit test command to include them. Otherwise those four tests
are explicitly skipped. They verify the rendered configuration has only public
80/443, loopback bridge ports, restricted-role initialization, the HTTPS origin,
persistent volumes and the optional container worker. Passing configuration
tests does not establish container startup or certificate issuance.

## Small-instance bootstrap

`deploy/ec2-bootstrap.sh` is credential-free Ubuntu 24.04 user data. It installs
Docker/Compose, creates a 4 GiB swap file on the encrypted root disk and prepares
a systemd service. Rust images use one build job and an identical workspace
build stage so the three independent runtime images can reuse compilation.
The production wrapper serializes builds. Before enabling the service, upload
only the reviewed source archive to `/opt/sanscue`, initialize runtime settings
with `sudo python3 -B deploy/configure-production.py`, build, and rehearse.
That initializer creates distinct secrets privately on first use, keeps them
on later boots, and reads EC2's current public IPv4 through IMDSv2 to refresh
the origin after stop/start. It never prints credentials. Enable/start
`sanscue.service` only after images are built; it does not perform image builds
at boot. The deployment-only SSH key and instance metadata stay in ignored
`local-data/ec2/`, not in Git. One-time EventBridge schedules were created and
read back enabled: stop October 24 at 00:00, start November 8 at 23:30, and stop
November 21 at 00:00, all America/Chicago in 2026. They target only SansCue;
their role permits starting/stopping only this instance. Future execution is
still subject to verification. Keep the Elastic IP reserved through judging.
