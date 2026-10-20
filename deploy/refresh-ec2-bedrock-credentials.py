#!/usr/bin/env python3
"""Refresh only this EC2 instance's Nova role credentials; never print secrets."""
import datetime
import json
import os
from pathlib import Path
import tempfile
import urllib.request

ROLE = "sanscue-demo-nova-micro"
ROOT = Path(__file__).resolve().parents[1]


def refresh():
    base = "http://169.254.169.254/latest/"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    request = urllib.request.Request(base + "api/token", method="PUT",
        headers={"X-aws-ec2-metadata-token-ttl-seconds": "60"})
    with opener.open(request, timeout=3) as response:
        token = response.read(4096).decode()
    headers = {"X-aws-ec2-metadata-token": token}
    request = urllib.request.Request(base + "meta-data/iam/security-credentials/" + ROLE,
                                     headers=headers)
    with opener.open(request, timeout=3) as response:
        value = json.loads(response.read(16384))
    if value.get("Code") != "Success" or not all(value.get(key) for key in
            ("AccessKeyId", "SecretAccessKey", "Token", "Expiration")):
        raise ValueError("role credentials unavailable")
    expires = datetime.datetime.fromisoformat(value["Expiration"].replace("Z", "+00:00"))
    if expires <= datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=5):
        raise ValueError("role credentials expire too soon")
    credentials = {"Version": 1, "AccessKeyId": value["AccessKeyId"],
                   "SecretAccessKey": value["SecretAccessKey"],
                   "SessionToken": value["Token"], "Expiration": value["Expiration"]}
    directory = ROOT / "local-data/bedrock"
    directory.mkdir(parents=True, exist_ok=True)
    # The runtime's UID is fixed; its primary GID is chosen by useradd and
    # need not equal the UID. Bind mounts are read-only in the container.
    os.chown(directory, 10003, 0)
    directory.chmod(0o700)
    fd, name = tempfile.mkstemp(prefix=".credentials-", dir=directory)
    staged = Path(name)
    try:
        with os.fdopen(fd, "w") as output:
            json.dump(credentials, output)
        os.chown(staged, 10003, 0)
        staged.chmod(0o600)
        staged.replace(directory / "credentials.json")
    finally:
        staged.unlink(missing_ok=True)


if __name__ == "__main__":
    try:
        refresh()
    except Exception:
        raise SystemExit("Bedrock credential refresh failed; existing credentials retained. Check the EC2 Nova role and IMDSv2 access.")
    print("Bedrock role credentials refreshed privately.")
