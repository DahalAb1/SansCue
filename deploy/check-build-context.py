#!/usr/bin/env python3
"""Check Docker's actual ignore behavior using only generated harmless fixtures."""
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ALLOWED = [
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml",
    "services/sessions/Cargo.toml", "services/sessions/src/main.rs",
    "services/sessions/src/nested/module.rs",
    "services/sessions/migrations/0001_bootstrap.sql",
    "services/bee-connection/Cargo.toml", "services/bee-connection/src/main.rs",
    "services/bee-connection/src/nested/module.rs",
    "services/bee-connection/migrations/0001_conversations.sql",
    "web/package.json", "web/package-lock.json", "web/index.html",
    "web/tsconfig.json", "web/vite.config.ts", "web/src/main.tsx",
    "web/src/app/App.tsx", "web/src/styles.css", "web/src/vite-env.d.ts",
    "deploy/Caddyfile",
]
BLOCKED_NAMES = [
    ".npmrc", ".netrc", ".pypirc", ".env", ".env.local",
    "identity.pem", "identity.key", "identity.p12", "identity.pfx",
    "identity.jks", "identity.keystore", "unknown-credentials.json",
    "private-backup.dump", "nested/unknown-credentials.json",
    ".aws/config", ".ssh/config", ".gnupg/config",
    "secrets/private.ts", "credentials/private.rs", "local-data/backup.sql",
    "node_modules/package/index.ts", "dist/bundle.ts",
]


def descendant_fixtures():
    # Negated paths can match directories, including exact manifest names.
    # Source-shaped children must also stay excluded by terminal rules.
    rules = (ROOT / ".dockerignore").read_text().splitlines()
    patterns = [line[1:] for line in rules if line.startswith("!")]
    last_allow = max(index for index, line in enumerate(rules) if line.startswith("!"))
    assert all(pattern + "/**" in rules[last_allow + 1:] for pattern in patterns), (
        "every allowlist entry needs a terminal descendant exclusion")
    directories = [pattern.replace("**/", "nested/").replace("*", "archive")
                   for pattern in patterns]
    return [directory + "/" + child for directory in directories for child in
            ("unknown-credentials.json", "private.ts", "private.tsx", "private.css",
             "private.rs", "private.sql", "nested/unknown-credentials.json",
             "nested/private.ts", "nested/private.rs", "nested/private.sql")]


def main():
    if not shutil.which("docker"):
        raise SystemExit("Missing prerequisite: Docker with BuildKit; ask the environment Fixer")
    with tempfile.TemporaryDirectory(prefix="sanscue-context-check-") as temporary:
        base = Path(temporary)
        context = base / "context"
        context.mkdir()
        # Never copy application directories or inspect actual credentials.
        blocked = [prefix + name for prefix in
                   ("", "web/", "web/src/", "services/", "services/sessions/",
                    "services/sessions/src/", "services/sessions/migrations/",
                    "services/bee-connection/", "services/bee-connection/src/",
                    "services/bee-connection/migrations/", "deploy/") for name in BLOCKED_NAMES]
        descendants = descendant_fixtures()
        for label, allowed, denied in (("normal", ALLOWED, blocked),
                                        ("directory-shaped", [], descendants)):
            case = context / label
            case.mkdir()
            shutil.copyfile(ROOT / ".dockerignore", case / ".dockerignore")
            for name in allowed + denied:
                path = case / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("harmless fixture\n")
            (case / "Dockerfile").write_text("FROM scratch\nCOPY . /\n")
            output = base / (label + "-output")
            subprocess.run(["docker", "build", "--no-cache", "--output",
                            f"type=local,dest={output}", str(case)], check=True, timeout=120)
            actual = {str(path.relative_to(output)) for path in output.rglob("*") if path.is_file()}
            assert actual == set(allowed), {"case": label, "missing": set(allowed) - actual,
                                           "unexpected": actual - set(allowed)}
            print(f"PASS ({label}): {len(allowed)} required inputs included; "
                  f"{len(denied)} credential/generated fixtures excluded")


if __name__ == "__main__":
    main()
