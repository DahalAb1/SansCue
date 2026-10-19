#!/bin/sh
# Keeps short-lived AWS credentials for Bedrock question generation fresh.
# Topics reads local-data/bedrock/credentials.json on every model request.
set -eu

profile=${1:-}
if [ -z "$profile" ]; then
  echo "usage: deploy/refresh-bedrock-credentials.sh AWS_PROFILE" >&2
  exit 1
fi
if ! command -v aws >/dev/null 2>&1; then
  echo "the AWS CLI is required" >&2
  exit 1
fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dir="$repo_root/local-data/bedrock"
mkdir -p "$dir"
chmod 700 "$dir"
umask 077

while :; do
  # Exported credentials last minutes, not hours; refresh well before expiry.
  if aws configure export-credentials --profile "$profile" --format process >"$dir/credentials.json.tmp"; then
    mv "$dir/credentials.json.tmp" "$dir/credentials.json"
  else
    rm -f "$dir/credentials.json.tmp"
    echo "could not refresh AWS credentials; run: aws login --profile $profile" >&2
  fi
  sleep 240
done
