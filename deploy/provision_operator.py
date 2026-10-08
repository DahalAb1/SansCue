#!/usr/bin/env python3
"""Local-only operator provisioning (stdlib only). Rerunning rotates everything.

Writes the credential's SHA-256, a unique random epoch, and an independent HMAC key
to deploy/.env, and the plaintext credential to local-data/operator/credential.
Secrets are never printed and never placed in argv or the environment.
"""
import argparse
import hashlib
import os
import re
import secrets
import sys
import tempfile
import time
from pathlib import Path

HASH_KEY = "SESSIONS_OPERATOR_TOKEN_SHA256"
EPOCH_KEY = "SESSIONS_OPERATOR_EPOCH"
HMAC_KEY = "SESSIONS_IDEMPOTENCY_HMAC_KEY"
DEFAULT_ROOT = Path(__file__).resolve().parents[1]


class ProvisionError(Exception):
    """Raised with a message that never contains a secret value."""


def _refuse_symlink(path):
    if path.is_symlink():
        raise ProvisionError(f"refusing symbolic link: {path}")


def _private_dir(path):
    _refuse_symlink(path)
    path.mkdir(mode=0o700, exist_ok=True)
    if not path.is_dir():
        raise ProvisionError(f"not a directory: {path}")
    os.chmod(path, 0o700)


def _fsync_dir(path):
    if not hasattr(os, "O_DIRECTORY"):
        return
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _stage(directory, prefix, data):
    """Write data to a new temp file beside its destination (explicitly mode 600)."""
    fd, name = tempfile.mkstemp(dir=directory, prefix=prefix, suffix=".tmp")
    try:
        with os.fdopen(fd, "wb") as handle:
            os.chmod(name, 0o600)
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
    except BaseException:
        Path(name).unlink(missing_ok=True)
        raise
    return Path(name)


def _read_env(env_path, example_path):
    if env_path.exists() or env_path.is_symlink():
        _refuse_symlink(env_path)
        return env_path.read_text(encoding="utf-8")
    if not example_path.is_file():
        raise ProvisionError(f"missing template: {example_path}")
    return example_path.read_text(encoding="utf-8")


def _assignment(key):
    return re.compile(rf"^\s*(?:export\s+)?{key}\s*=(.*)$")


def current_epoch(text):
    match = None
    for line in text.splitlines():
        found = _assignment(EPOCH_KEY).match(line)
        if found:
            match = found.group(1).strip().strip("\"'")
    return match


def set_values(text, values):
    """Replace the first assignment of each key, drop duplicates, append missing keys."""
    patterns = {key: _assignment(key) for key in values}
    seen = set()
    lines = []
    for line in text.splitlines():
        key = next((k for k, p in patterns.items() if p.match(line)), None)
        if key is None:
            lines.append(line)
        elif key not in seen:
            seen.add(key)
            lines.append(f"{key}={values[key]}")
    missing = [key for key in values if key not in seen]
    if missing and lines and lines[-1] != "":
        lines.append("")
    lines.extend(f"{key}={values[key]}" for key in missing)
    return "\n".join(lines) + "\n"


def new_epoch(previous):
    """UTC time for readability plus 128 random bits so rapid rotations never collide."""
    while True:
        epoch = f"{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}-{secrets.token_hex(16)}"
        if epoch != previous:
            return epoch


def provision(root=DEFAULT_ROOT):
    root = Path(root)
    deploy = root / "deploy"
    env_path = deploy / ".env"
    local_data = root / "local-data"
    operator_dir = local_data / "operator"
    credential_path = operator_dir / "credential"
    if not deploy.is_dir():
        raise ProvisionError(f"missing directory: {deploy}")
    _refuse_symlink(credential_path)

    env_text = _read_env(env_path, deploy / ".env.example")
    credential = secrets.token_hex(32)
    values = {
        HASH_KEY: hashlib.sha256(credential.encode("ascii")).hexdigest(),
        EPOCH_KEY: new_epoch(current_epoch(env_text)),
        HMAC_KEY: secrets.token_hex(32),
    }
    new_env = set_values(env_text, values).encode("utf-8")

    _private_dir(local_data)
    _private_dir(operator_dir)
    previous_credential = credential_path.read_bytes() if credential_path.exists() else None
    credential_tmp = env_tmp = None
    try:
        credential_tmp = _stage(operator_dir, ".credential.", credential.encode("ascii"))
        env_tmp = _stage(deploy, ".env.", new_env)
        os.replace(credential_tmp, credential_path)
        credential_tmp = None
        try:
            os.replace(env_tmp, env_path)
            env_tmp = None
        except BaseException:
            # Keep the credential and hash in step if the second replace fails.
            if previous_credential is None:
                credential_path.unlink(missing_ok=True)
            else:
                restore = _stage(operator_dir, ".credential.", previous_credential)
                os.replace(restore, credential_path)
            raise
        _fsync_dir(operator_dir)
        _fsync_dir(deploy)
    finally:
        for temp in (credential_tmp, env_tmp):
            if temp is not None:
                temp.unlink(missing_ok=True)
    return env_path, credential_path


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT, help="repository root (default: this checkout)")
    args = parser.parse_args(argv)
    os.umask(0o077)
    try:
        provision(args.root)
    except (ProvisionError, OSError) as error:
        print(f"provisioning failed: {error.__class__.__name__}: {error}", file=sys.stderr)
        return 1
    print("Updated deploy/.env (operator hash, epoch, HMAC key; mode 600).")
    print("Wrote local-data/operator/credential (mode 600). Values are not printed.")
    print("Recreate sessions to apply: docker compose --env-file deploy/.env up -d --wait --wait-timeout 120 sessions")
    return 0


if __name__ == "__main__":
    sys.exit(main())
