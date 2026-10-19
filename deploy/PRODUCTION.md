# EC2 deployment preparation

Status: configuration prepared; no EC2 resources have been created and no hosted
HTTPS acceptance is claimed. Local live/Compose results are recorded in
[environment verification](../docs/environment-verification.md). Use a staging
deployment with consented test speech before inviting an external audience.

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
- A stable public address and a domain/subdomain with DNS pointing to it. Do
  not publish an AAAA record unless IPv6 reaches the instance.
- Security group: public inbound TCP 80 and 443; SSH TCP 22 from the trusted
  administrator/Mac IP only. No inbound database or service ports. Verify the
  host firewall as well. Keep SSH keys private and verify the server host key.
- A separate checkout/host for staging. Named production volumes are distinct
  from local development; do not reuse existing demo database volumes.

Caddy manages certificates and HTTP-to-HTTPS redirects; `/data` is persisted
across container recreation. See [Caddy HTTPS requirements](https://caddyserver.com/docs/automatic-https).
The production override uses Docker's [replacement/reset semantics](https://docs.docker.com/reference/compose-file/merge/)
to remove the local web/DB port configuration.

## Configure on EC2

Use a release revision reviewed by the team; record `git rev-parse HEAD`. Work
in a private checkout whose access is limited to trusted deployment operators.

```sh
umask 077
cp deploy/.env.production.example deploy/.env
chmod 600 deploy/.env
```

Edit the private file locally on EC2: set `SANSCUE_DOMAIN` to the bare lowercase
hostname, `ACME_EMAIL` to your contact address, and fill all database passwords,
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
Email organizer login is proposed, not implemented; do not email this shared
operator credential as a substitute for a one-time login code.

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

Configure at most one provider in EC2's private `deploy/.env`. All blank runs
the clearly labeled placeholder; a transcript reaching the UI does not prove
AI generation. For an initial live Bedrock rehearsal, configure a valid Bedrock
API key only if the AWS organization permits it. The existing file-based
short-lived credential refresh path requires a host refresher on EC2 and read
access for Topics' UID 10003; the Mac's local credential file is not available
on EC2. Do not assume an EC2 IAM role is used automatically: the current
generator supports only its configured API key or credential-file paths.
Verify quota, a successful evidenced draft, latency and cost before the demo.

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

## Organizer email login proposal

The current single host-operator credential grants room creation and can
restore speaker access across rooms. Email access therefore needs a deliberate
authorization change, not just a mail-sending button. Pending product choices:
one-time code versus sign-in link, approved organizer addresses, and email
provider/domain. Recommended flow: approved organizer requests access, receives
a short-lived single-use code, submits it on the HTTPS page, and receives a
Secure HttpOnly session. Store only a keyed hash of a short code, limit requests
and guesses across sessions, consume atomically, and return generic request
responses. Each organizer should own their rooms rather than inherit global
operator access. Keep bootstrap operator access restricted to the host.

AWS SES is one possible provider; sandbox accounts can send only to verified
recipients (or the mailbox simulator) until production access is granted. See
[SES production access](https://docs.aws.amazon.com/ses/latest/dg/request-production-access.html).
Email login and outbound email sending have not been implemented or executed.

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
