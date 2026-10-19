import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('configure_production', ROOT/'deploy/configure-production.py')
configure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(configure)

class ConfigureProduction(unittest.TestCase):
    def test_initialization_is_private_and_boot_refresh_preserves_secrets(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            (root/'deploy').mkdir()
            (root/'deploy/.env.production.example').write_text((ROOT/'deploy/.env.production.example').read_text())
            configure.configure(root,'8.8.8.8')
            env=root/'deploy/.env'
            initial=env.read_text()
            credential=(root/'local-data/operator/credential').read_bytes()
            self.assertEqual(env.stat().st_mode & 0o777,0o600)
            self.assertIn('SESSIONS_DEMO_OPEN_ROOMS=true',initial)
            self.assertNotIn(credential.decode(),initial)
            configure.configure(root,'1.1.1.1')
            self.assertEqual(env.read_text(),initial.replace('SANSCUE_DOMAIN=8.8.8.8','SANSCUE_DOMAIN=1.1.1.1'))
            self.assertEqual((root/'local-data/operator/credential').read_bytes(),credential)

    def test_private_ips_are_rejected_before_writing(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            for value in ('127.0.0.1','10.0.0.1','169.254.169.254'):
                with self.assertRaises(ValueError): configure.configure(root,value)
            self.assertEqual(list(root.iterdir()),[])
