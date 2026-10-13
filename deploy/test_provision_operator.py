#!/usr/bin/env python3
"""Stdlib tests for deploy/provision_operator.py. They only touch temporary directories."""
import contextlib
import hashlib
import io
import os
import re
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import provision_operator as po  # noqa: E402

EXAMPLE = (HERE / ".env.example").read_text()
HEX64 = re.compile(r"^[0-9a-f]{64}$")
POSIX = os.name == "posix"


def mode(path):
    return stat.S_IMODE(os.stat(path).st_mode)


def read_env(path):
    values = {}
    for line in Path(path).read_text().splitlines():
        if "=" in line and not line.startswith("#"):
            key, value = line.split("=", 1)
            values[key] = value
    return values


class ProvisionOperatorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "deploy").mkdir()
        (self.root / "deploy/.env.example").write_text(EXAMPLE)
        self.env = self.root / "deploy/.env"
        self.credential = self.root / "local-data/operator/credential"

    def run_once(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            self.assertEqual(po.main(["--root", str(self.root)]), 0)
        values = read_env(self.env)
        return values, self.credential.read_text(), out.getvalue()

    def test_values_are_consistent_and_well_formed(self):
        values, credential, output = self.run_once()
        self.assertRegex(credential, HEX64)  # exactly the credential, no newline
        self.assertEqual(values[po.HASH_KEY], hashlib.sha256(credential.encode()).hexdigest())
        self.assertRegex(values[po.HMAC_KEY], HEX64)
        self.assertRegex(values[po.EPOCH_KEY], r"^[A-Za-z0-9._-]+$")
        self.assertRegex(values[po.EPOCH_KEY], r"-[0-9a-f]{32}$")  # random, not timestamp-only
        self.assertEqual(len({credential, values[po.HASH_KEY], values[po.HMAC_KEY]}), 3)
        self.assertNotIn(credential, values.values())
        self.assertNotIn(credential, self.env.read_text())
        self.assertNotIn(credential, output)

    def test_preserves_other_settings_and_comments(self):
        self.run_once()
        text = self.env.read_text()
        self.assertIn("POSTGRES_DB=sessions\n", text)
        self.assertIn("POSTGRES_PASSWORD=sanscue-local-only-change-me\n", text)
        self.assertIn("# Operator access", text)
        for key in (po.HASH_KEY, po.EPOCH_KEY, po.HMAC_KEY):
            self.assertEqual(len(re.findall(rf"^{key}=", text, re.M)), 1)

    def test_existing_env_is_updated_in_place_and_duplicates_collapse(self):
        self.env.write_text(f"A=1\n{po.EPOCH_KEY}=old\nB=2\nexport {po.EPOCH_KEY}=older\n")
        values, _, _ = self.run_once()
        text = self.env.read_text()
        self.assertTrue(text.startswith("A=1\n"))
        self.assertIn("B=2\n", text)
        self.assertEqual(len(re.findall(rf"{po.EPOCH_KEY}=", text)), 1)
        self.assertEqual(set(values), {"A", "B", po.EPOCH_KEY, po.HASH_KEY, po.HMAC_KEY})

    def test_rotations_are_unique_even_when_rapid(self):
        seen = {"credential": set(), "hash": set(), "hmac": set(), "epoch": set()}
        for _ in range(25):
            values, credential, _ = self.run_once()
            seen["credential"].add(credential)
            seen["hash"].add(values[po.HASH_KEY])
            seen["hmac"].add(values[po.HMAC_KEY])
            seen["epoch"].add(values[po.EPOCH_KEY])
        self.assertTrue(all(len(v) == 25 for v in seen.values()), {k: len(v) for k, v in seen.items()})

    def test_same_second_epochs_differ(self):
        original = po.time.strftime
        po.time.strftime = lambda *_: "20260101T000000Z"
        self.addCleanup(setattr, po.time, "strftime", original)
        epochs = {po.new_epoch(None) for _ in range(50)}
        self.assertEqual(len(epochs), 50)
        self.assertNotIn(po.new_epoch("x"), {"x"})

    def test_new_epoch_never_repeats_previous(self):
        calls = iter(["a" * 32, "a" * 32, "b" * 32])
        original = po.secrets.token_hex, po.time.strftime
        po.secrets.token_hex = lambda _n: next(calls)
        po.time.strftime = lambda *_: "20260101T000000Z"
        self.addCleanup(lambda: (setattr(po.secrets, "token_hex", original[0]), setattr(po.time, "strftime", original[1])))
        previous = f"20260101T000000Z-{'a' * 32}"
        self.assertTrue(po.new_epoch(previous).endswith("b" * 32))

    @unittest.skipUnless(POSIX, "POSIX permissions")
    def test_permissions(self):
        old = os.umask(0o022)  # a permissive umask must not weaken anything
        self.addCleanup(os.umask, old)
        po.provision(self.root)
        self.assertEqual(mode(self.env), 0o600)
        self.assertEqual(mode(self.credential), 0o600)
        self.assertEqual(mode(self.root / "local-data"), 0o700)
        self.assertEqual(mode(self.root / "local-data/operator"), 0o700)

    @unittest.skipUnless(POSIX, "POSIX permissions")
    def test_rotation_tightens_existing_loose_modes(self):
        (self.root / "local-data/operator").mkdir(parents=True)
        os.chmod(self.root / "local-data", 0o755)
        os.chmod(self.root / "local-data/operator", 0o755)
        self.env.write_text(EXAMPLE)
        os.chmod(self.env, 0o644)
        self.run_once()
        self.assertEqual(mode(self.env), 0o600)
        self.assertEqual(mode(self.root / "local-data"), 0o700)
        self.assertEqual(mode(self.root / "local-data/operator"), 0o700)

    def test_stdout_and_stderr_carry_no_secret_values(self):
        for _ in range(3):
            result = subprocess.run(
                [sys.executable, "-B", str(HERE / "provision_operator.py"), "--root", str(self.root)],
                capture_output=True, text=True, check=True, env={"PATH": os.environ.get("PATH", "")})
            values = read_env(self.env)
            secrets_ = [self.credential.read_text(), values[po.HASH_KEY], values[po.HMAC_KEY], values[po.EPOCH_KEY]]
            for text in (result.stdout, result.stderr):
                self.assertNotRegex(text, r"[0-9a-f]{32}")
                for secret in secrets_:
                    self.assertNotIn(secret, text)
            self.assertTrue(result.stdout.strip())

    def test_no_temp_files_remain(self):
        self.run_once()
        self.run_once()
        self.assertEqual(sorted(p.name for p in (self.root / "deploy").iterdir()), [".env", ".env.example"])
        self.assertEqual([p.name for p in self.credential.parent.iterdir()], ["credential"])

    def test_failure_leaves_previous_state_and_leaks_nothing(self):
        values, credential, _ = self.run_once()
        before = self.env.read_text()
        original = po.os.replace
        calls = []

        def flaky(src, dst):
            calls.append(dst)
            if len(calls) == 2:  # the .env replace
                raise OSError("simulated failure")
            return original(src, dst)

        po.os.replace = flaky
        self.addCleanup(setattr, po.os, "replace", original)
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            self.assertEqual(po.main(["--root", str(self.root)]), 1)
        po.os.replace = original
        self.assertEqual(self.env.read_text(), before)
        self.assertEqual(self.credential.read_text(), credential)
        self.assertNotIn(credential, err.getvalue())
        self.assertEqual(sorted(p.name for p in (self.root / "deploy").iterdir()), [".env", ".env.example"])

    @unittest.skipUnless(POSIX, "symlinks")
    def test_refuses_symlinked_targets(self):
        target = self.root / "elsewhere"
        target.write_text("keep")
        self.env.symlink_to(target)
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(po.main(["--root", str(self.root)]), 1)
        self.assertEqual(target.read_text(), "keep")

    def test_missing_template_fails_closed(self):
        (self.root / "deploy/.env.example").unlink()
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(po.main(["--root", str(self.root)]), 1)
        self.assertFalse(self.credential.exists())


if __name__ == "__main__":
    unittest.main()
