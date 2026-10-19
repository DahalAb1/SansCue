#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
env_file=${SANSCUE_WORKER_ENV_FILE:-"$repo_root/deploy/.env"}
worker="$repo_root/target/debug/bee-connection"

if [ ! -f "$env_file" ]; then
  echo "a trusted worker env file is required; see deploy/PRODUCTION.md for remote use" >&2
  exit 1
fi
if [ ! -x "$worker" ]; then
  echo "build bee-connection before starting the live worker" >&2
  exit 1
fi

set -a
# This is a trusted, private shell configuration file, never an uploaded file.
. "$env_file"
set +a

: "${BEE_POSTGRES_USER:?Set BEE_POSTGRES_USER in deploy/.env}"
: "${BEE_POSTGRES_PASSWORD:?Set BEE_POSTGRES_PASSWORD in deploy/.env}"
: "${BEE_POSTGRES_DB:?Set BEE_POSTGRES_DB in deploy/.env}"
: "${SESSIONS_BEE_SERVICE_TOKEN:?Set SESSIONS_BEE_SERVICE_TOKEN in deploy/.env}"
: "${TOPICS_BEE_SERVICE_TOKEN:?Set TOPICS_BEE_SERVICE_TOKEN in deploy/.env}"

bee_cli=${BEE_CLI_PATH:-}
if [ -z "$bee_cli" ]; then
  bee_cli=$(command -v bee || true)
fi
if [ -z "$bee_cli" ] || [ ! -x "$bee_cli" ]; then
  echo "an authenticated Bee CLI is required (set BEE_CLI_PATH if needed)" >&2
  exit 1
fi

export BEE_CLI_PATH="$bee_cli"
export BEE_DATABASE_URL="postgresql://${BEE_POSTGRES_USER}:${BEE_POSTGRES_PASSWORD}@127.0.0.1:55433/${BEE_POSTGRES_DB}"
export SESSIONS_INTERNAL_URL="http://127.0.0.1:3401"
export TOPICS_INTERNAL_URL="http://127.0.0.1:3402"

exec "$worker" --worker
