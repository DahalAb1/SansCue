#!/usr/bin/env python3
"""Validate private production settings without printing their values (stdlib only)."""
import os
from pathlib import Path
import re
import stat
import sys

ROOT = Path(__file__).resolve().parents[1]
DATABASES = ("POSTGRES", "BEE_POSTGRES", "TOPICS_POSTGRES")
TOKENS = ("SESSIONS_BEE_SERVICE_TOKEN", "SESSIONS_TOPICS_SERVICE_TOKEN",
          "TOPICS_BEE_SERVICE_TOKEN", "TOPICS_SESSIONS_SERVICE_TOKEN")
KEYS = ("SESSIONS_OPERATOR_TOKEN_SHA256", "SESSIONS_IDEMPOTENCY_HMAC_KEY")
BOOTSTRAP = ("SESSIONS_DB_BOOTSTRAP_PASSWORD", "BEE_DB_BOOTSTRAP_PASSWORD",
             "TOPICS_DB_BOOTSTRAP_PASSWORD")


def read_settings(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError("deploy/.env must be a regular file, not a symlink")
    if stat.S_IMODE(path.stat().st_mode) & 0o077:
        raise ValueError("deploy/.env must have mode 600 or stricter")
    settings = {}
    for line in path.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        match = re.fullmatch(r"([A-Z][A-Z0-9_]*)=([^\s\"'#$`]*)", line)
        if not match:
            raise ValueError("deploy/.env requires plain KEY=value lines without interpolation or quotes")
        key, value = match.groups()
        if key in settings:
            raise ValueError(f"duplicate setting: {key}")
        settings[key] = value
    # Shell variables override --env-file in Compose. Reject rather than validate
    # one value and silently deploy another, even if the two currently match.
    if set(settings) & set(os.environ):
        raise ValueError("unset exported deployment settings before using the production wrapper")
    return settings


def validate(settings):
    errors = []
    domain = settings.get("SANSCUE_DOMAIN", "")
    labels = domain.split(".")
    if (len(domain) > 253 or len(labels) < 2
            or any(not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", label) for label in labels)
            or labels[-1] in {"localhost", "local", "test", "invalid", "example"}
            or domain in {"example.com", "example.org", "example.net"}
            or not re.fullmatch(r"[a-z]{2,63}", labels[-1])):
        errors.append("SANSCUE_DOMAIN must be a real public DNS hostname in lowercase")
    if not re.fullmatch(r"[^@\s]+@[^@\s]+\.[^@\s]+", settings.get("ACME_EMAIL", "")):
        errors.append("ACME_EMAIL is required")
    secrets = []
    for prefix in DATABASES:
        for suffix in ("DB", "USER"):
            key = f"{prefix}_{suffix}"
            if (not re.fullmatch(r"[a-z][a-z0-9_]{0,62}", settings.get(key, ""))
                    or settings.get(key) == "postgres"):
                errors.append(f"{key} must be a lowercase database identifier")
        key = f"{prefix}_PASSWORD"
        value = settings.get(key, "")
        if not re.fullmatch(r"[A-Za-z0-9._~-]{32,}", value):
            errors.append(f"{key} requires at least 32 random URL-unreserved characters")
        secrets.append(value)
    for key in BOOTSTRAP:
        value = settings.get(key, "")
        if not re.fullmatch(r"[A-Za-z0-9._~-]{32,}", value):
            errors.append(f"{key} requires an independent random password of at least 32 characters")
        secrets.append(value)
    for key in TOKENS:
        value = settings.get(key, "")
        if not re.fullmatch(r"[A-Za-z0-9._~-]{32,}", value) or "local-dev" in value:
            errors.append(f"{key} requires an independent random credential of at least 32 characters")
        secrets.append(value)
    for key in KEYS:
        value = settings.get(key, "")
        if not re.fullmatch(r"[0-9a-fA-F]{64}", value):
            errors.append(f"{key} requires 64 hex characters; run operator provisioning")
        secrets.append(value.lower())
    if len(set(secrets)) != len(secrets):
        errors.append("database passwords, service tokens, operator hash and HMAC key must be distinct")
    if not re.fullmatch(r"[A-Za-z0-9._-]{1,100}", settings.get("SESSIONS_OPERATOR_EPOCH", "")):
        errors.append("SESSIONS_OPERATOR_EPOCH is required")
    providers = [key for key in ("ANTHROPIC_API_KEY", "AWS_BEARER_TOKEN_BEDROCK",
                                "TOPICS_BEDROCK_CREDENTIALS_FILE") if settings.get(key)]
    if len(providers) > 1:
        errors.append("configure at most one model provider credential")
    return errors


def main():
    try:
        errors = validate(read_settings(ROOT / "deploy/.env"))
    except (OSError, ValueError):
        # Do not echo exception text from file parsing: it can contain secrets.
        print("Production settings could not be read: use a private regular deploy/.env, plain KEY=value syntax, and unset exported deployment variables.", file=sys.stderr)
        return 1
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("Production settings passed; values were not printed. Runtime checks are separate.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
