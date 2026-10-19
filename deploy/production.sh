#!/bin/sh
# Always use the same project/configuration, including for backup and stop commands.
set -eu
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
python3 -B deploy/check-production.py
exec docker compose --parallel 1 --project-name sanscue-production --env-file deploy/.env \
  -f compose.yaml -f compose.production.yaml "$@"
