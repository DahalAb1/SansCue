import datetime
import importlib.util
import io
import json
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("ec2_refresh", Path(__file__).with_name("refresh-ec2-bedrock-credentials.py"))
refresh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(refresh)


class CredentialRefresh(unittest.TestCase):
    def test_private_atomic_refresh_and_failed_refresh_preserves_last_value(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            value = {"Code":"Success", "AccessKeyId":"test-key", "SecretAccessKey":"test-secret",
                     "Token":"test-token", "Expiration":(datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(hours=1)).isoformat()}
            class Opener:
                count = 0
                def open(self, request, timeout):
                    self.count += 1
                    if self.count % 2 == 1:
                        self_test.assertEqual(request.method, "PUT")
                        return io.BytesIO(b"imds-test-token")
                    self_test.assertEqual(request.full_url, "http://169.254.169.254/latest/meta-data/iam/security-credentials/sanscue-demo-nova-micro")
                    return io.BytesIO(json.dumps(value).encode())
            self_test = self
            with patch.object(refresh, "ROOT", root), patch.object(refresh.urllib.request, "build_opener", return_value=Opener()), patch.object(refresh.os, "chown") as chown:
                refresh.refresh()
                target = root / "local-data/bedrock/credentials.json"
                initial = target.read_bytes()
                self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o600)
                self.assertEqual(stat.S_IMODE(target.parent.stat().st_mode), 0o700)
                self.assertEqual(json.loads(initial)["SessionToken"], "test-token")
                self.assertEqual(chown.call_args.args[1:], (10003,0))
                value["Expiration"] = "2000-01-01T00:00:00Z"
                with self.assertRaises(ValueError):refresh.refresh()
                self.assertEqual(target.read_bytes(), initial)
                self.assertEqual(list(target.parent.iterdir()), [target])
