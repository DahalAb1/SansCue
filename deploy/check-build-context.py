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


def main():
    if not shutil.which("docker"):
        raise SystemExit("Missing prerequisite: Docker with BuildKit; ask the environment Fixer")
    with tempfile.TemporaryDirectory(prefix="sanscue-context-check-") as temporary:
        base = Path(temporary)
        context = base / "context"
        context.mkdir()
        # Never copy application directories or inspect actual credentials.
        shutil.copyfile(ROOT / ".dockerignore", context / ".dockerignore")
        blocked = [prefix + name for prefix in
                   ("", "web/", "web/src/", "services/", "services/sessions/",
                    "services/sessions/src/", "services/sessions/migrations/", "deploy/") for name in BLOCKED_NAMES]
        for name in ALLOWED + blocked:
            path = context / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("harmless fixture\n")
        (context / "Dockerfile").write_text("FROM scratch\nCOPY . /\n")
        output = base / "output"
        subprocess.run(["docker", "build", "--no-cache", "--output",
                        f"type=local,dest={output}", str(context)], check=True, timeout=120)
        actual = {str(path.relative_to(output)) for path in output.rglob("*") if path.is_file()}
        assert actual == set(ALLOWED), {"missing": set(ALLOWED) - actual,
                                       "unexpected": actual - set(ALLOWED)}
        print(f"PASS: {len(ALLOWED)} required inputs included; {len(blocked)} credential/generated fixtures excluded")


if __name__ == "__main__":
    main()
