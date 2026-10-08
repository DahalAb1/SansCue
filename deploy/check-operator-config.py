#!/usr/bin/env python3
"""Static checks for operator configuration pass-through. Needs neither Docker nor secrets."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
NAMES = ["SESSIONS_OPERATOR_TOKEN_SHA256", "SESSIONS_OPERATOR_EPOCH",
         "SESSIONS_IDEMPOTENCY_HMAC_KEY"]
compose = (ROOT / "compose.yaml").read_text()
example = (ROOT / "deploy/.env.example").read_text()
readme = (ROOT / "deploy/README.md").read_text()
sessions = compose.split("  sessions:\n", 1)[1].split("\n  sessions-db:", 1)[0]

for name in NAMES:
    # Blank default keeps the operator disabled; no weak default and no hard requirement.
    assert re.search(rf'^\s+{name}: "\${{{name}:-}}"$', sessions, re.M), name
    assert compose.count(name) == 2, f"{name} must only reach sessions"
    # Example holds a blank placeholder, never a value.
    assert re.search(rf"^{name}=$", example, re.M), name
    assert name in readme, name
assert "args:" not in compose and "build:\n      context" in compose, "no build-time secrets"

for line in example.splitlines():
    if line.startswith(tuple(NAMES)):
        assert line.split("=", 1)[1] == "", "example must not contain values"
    assert not re.search(r"\b[0-9a-f]{64}\b", line), "example must not contain key-like values"

for phrase in ["create rooms", "fail closed", "256-bit", "openssl rand -hex 32",
               "sha256sum", "never", "Git-ignored"]:
    assert phrase.lower() in readme.lower(), phrase
assert not re.search(r"\b[0-9a-f]{64}\b", readme), "README must not contain key-like values"
print("operator configuration static checks passed")
