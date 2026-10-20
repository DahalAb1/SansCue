#!/usr/bin/env python3
"""Initialize EC2 runtime secrets privately and refresh its public-IP origin on boot."""
import argparse
import ipaddress
from pathlib import Path
import secrets
import urllib.request

import provision_operator as operator


def metadata_ip():
    # Fixed EC2 metadata endpoint, IMDSv2 only; no arbitrary URL input.
    request = urllib.request.Request("http://169.254.169.254/latest/api/token", method="PUT",
                                     headers={"X-aws-ec2-metadata-token-ttl-seconds": "60"})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(request, timeout=3) as response:
        token = response.read(4096).decode()
    request = urllib.request.Request("http://169.254.169.254/latest/meta-data/public-ipv4",
                                     headers={"X-aws-ec2-metadata-token": token})
    with opener.open(request, timeout=3) as response:
        return response.read(128).decode().strip()


def configure(root, public_ip):
    address = ipaddress.ip_address(public_ip)
    if address.version != 4 or not address.is_global:
        raise ValueError("a public IPv4 address is required")
    env = root / "deploy/.env"
    text = operator._read_env(env, root / "deploy/.env.production.example")
    current = {}
    for line in text.splitlines():
        if line and not line.startswith("#") and "=" in line:
            name, value = line.split("=", 1)
            current[name] = value
    values = {"SANSCUE_DOMAIN": str(address)}
    for name in ("POSTGRES_PASSWORD", "BEE_POSTGRES_PASSWORD", "TOPICS_POSTGRES_PASSWORD",
                 "SESSIONS_DB_BOOTSTRAP_PASSWORD", "BEE_DB_BOOTSTRAP_PASSWORD", "TOPICS_DB_BOOTSTRAP_PASSWORD",
                 "SESSIONS_BEE_SERVICE_TOKEN", "SESSIONS_TOPICS_SERVICE_TOKEN", "TOPICS_BEE_SERVICE_TOKEN",
                 "TOPICS_SESSIONS_SERVICE_TOKEN"):
        if not current.get(name):
            values[name] = secrets.token_hex(32)
    staged = operator._stage(env.parent, ".env.", operator.set_values(text, values).encode())
    try:
        staged.replace(env)
    finally:
        staged.unlink(missing_ok=True)
    if not all(current.get(name) for name in (operator.HASH_KEY, operator.EPOCH_KEY, operator.HMAC_KEY)):
        operator.provision(root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--public-ip", help="public IPv4 address; omitted reads EC2 IMDSv2")
    args = parser.parse_args()
    try:
        configure(Path(__file__).resolve().parents[1], args.public_ip or metadata_ip())
    except (OSError, ValueError):
        raise SystemExit("Production initialization failed; verify metadata access, the public IP and private runtime files.")
    print("Production settings initialized; secrets were not printed.")


if __name__ == "__main__":
    main()
