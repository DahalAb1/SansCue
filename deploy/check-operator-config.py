#!/usr/bin/env python3
"""Static checks for operator configuration pass-through. Needs neither Docker nor secrets."""
import ast
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

for phrase in ["create rooms", "fail closed", "256-bit", "python3 deploy/provision_operator.py",
               "unique random epoch", "rerunning rotates", "read the credential privately",
               "local-data/operator/credential", "mode 600", "never", "Git-ignored",
               "unittest discover -s deploy"]:
    assert phrase.lower() in readme.lower(), phrase
assert not re.search(r"\b[0-9a-f]{64}\b", readme), "README must not contain key-like values"

# The old shell recipe put values in argv (sed) and used a timestamp-only epoch.
for stale in ["sed -e", "openssl rand", "sha256sum", "date -u +%Y%m%dT%H%M%SZ)\n", "deploy/.env.new",
              "op_hash", "hmac_key="]:
    assert stale not in readme, f"stale shell provisioning recipe: {stale}"

# Secrets stay local: ignored by Git and denied from every Docker build context.
gitignore = (ROOT / ".gitignore").read_text().splitlines()
dockerignore = (ROOT / ".dockerignore").read_text().splitlines()
for pattern in [".env", ".env.*", "local-data/"]:
    assert pattern in gitignore, f".gitignore must ignore {pattern}"
for pattern in ["**", "**/.env", "**/.env.*", "**/local-data"]:
    assert pattern in dockerignore, f".dockerignore must deny {pattern}"
assert "!deploy/.env" not in dockerignore and not any(l.startswith("!local-data") for l in dockerignore)

# The provisioning utility: stdlib only, no argv/environment secrets, no subprocesses.
script_text = (ROOT / "deploy/provision_operator.py").read_text()
tree = ast.parse(script_text)
imported = {n.name.split(".")[0] for x in ast.walk(tree) if isinstance(x, ast.Import) for n in x.names}
imported |= {x.module.split(".")[0] for x in ast.walk(tree) if isinstance(x, ast.ImportFrom) and x.module}
allowed = {"argparse", "hashlib", "os", "re", "secrets", "sys", "tempfile", "time", "pathlib"}
assert imported <= allowed, f"unexpected import: {imported - allowed}"
assert "os.environ" not in script_text and "getenv" not in script_text, "no environment secrets"
for token in ["secrets.token_hex(32)", "hashlib.sha256", "os.replace", "mkstemp", "0o600", "0o700"]:
    assert token in script_text, token
for call in (x for x in ast.walk(tree) if isinstance(x, ast.Call)):
    func = getattr(call.func, "id", "")
    if func == "print":
        for node in ast.walk(call):
            assert not (isinstance(node, ast.Name) and node.id in {"credential", "values", "new_env", "env_text"}), \
                "print must not reference secret values"
assert (ROOT / "deploy/test_provision_operator.py").is_file()
print("operator configuration static checks passed")
