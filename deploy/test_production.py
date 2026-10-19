"""Production settings and actual Compose merge checks; no live credentials or AWS."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("production_check", ROOT / "deploy/check-production.py")
production = importlib.util.module_from_spec(spec)
spec.loader.exec_module(production)


def fixture():
    values = {
        "SANSCUE_DOMAIN": "sanscue.acme.org",
        "SESSIONS_OPERATOR_EPOCH": "test-deployment-1",
    }
    for prefix, db in zip(production.DATABASES, ("sessions", "bee", "topics")):
        values[f"{prefix}_DB"] = db
        values[f"{prefix}_USER"] = db + "_app"
        values[f"{prefix}_PASSWORD"] = hashlib.sha256(prefix.encode()).hexdigest()
    for key in (*production.TOKENS, *production.KEYS, *production.BOOTSTRAP):
        values[key] = hashlib.sha256(key.encode()).hexdigest()
    return values


def write_fixture(path, values):
    path.write_text("".join(f"{key}={value}\n" for key, value in values.items()))
    path.chmod(0o600)


class ProductionSettings(unittest.TestCase):
    def test_complete_settings(self):
        self.assertEqual(production.validate(fixture()), [])

    def test_public_ip_and_demo_mode(self):
        settings = fixture()
        settings["SANSCUE_DOMAIN"] = "8.8.8.8"
        settings["SESSIONS_DEMO_OPEN_ROOMS"] = "true"
        self.assertEqual(production.validate(settings), [])
        settings["SESSIONS_DEMO_OPEN_ROOMS"] = "yes"
        self.assertTrue(production.validate(settings))
        for address in ("127.0.0.1", "10.0.0.1", "192.168.1.1", "0.0.0.0", "169.254.169.254"):
            settings["SANSCUE_DOMAIN"] = address
            self.assertTrue(production.validate(settings))

    def test_missing_secret_and_local_defaults_are_rejected(self):
        for key in (*production.TOKENS, *production.KEYS, *production.BOOTSTRAP,
                    *(prefix + "_PASSWORD" for prefix in production.DATABASES)):
            for value in ("", "local-dev-do-not-use"):
                settings = fixture()
                settings[key] = value
                self.assertTrue(production.validate(settings), key)

    def test_domain_must_be_bare_public_hostname(self):
        for value in ("", "https://sanscue.acme.org", "sanscue.acme.org/", "127.0.0.1",
                      "localhost", "demo.local", "example.com", "*.acme.org", "-bad.acme.org"):
            settings = fixture()
            settings["SANSCUE_DOMAIN"] = value
            self.assertTrue(production.validate(settings), value)

    def test_secrets_are_independent(self):
        settings = fixture()
        settings["BEE_POSTGRES_PASSWORD"] = settings["POSTGRES_PASSWORD"]
        self.assertTrue(production.validate(settings))

    def test_bootstrap_role_is_not_application_role(self):
        for suffix in ("USER", "DB"):
            settings = fixture()
            settings["POSTGRES_" + suffix] = "postgres"
            self.assertTrue(production.validate(settings))

    def test_multiple_providers_rejected(self):
        settings = fixture()
        settings.update(ANTHROPIC_API_KEY="fake-a", AWS_BEARER_TOKEN_BEDROCK="fake-b")
        self.assertTrue(production.validate(settings))

    def test_errors_do_not_echo_values(self):
        settings = fixture()
        secret = "invalid-secret-do-not-echo!"
        settings["POSTGRES_PASSWORD"] = secret
        self.assertNotIn(secret, "\n".join(production.validate(settings)))

    def test_private_file_and_shell_override_checks(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {}, clear=True):
            path = Path(directory) / ".env"
            write_fixture(path, fixture())
            self.assertEqual(production.read_settings(path), fixture())
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                production.read_settings(path)
            path.chmod(0o600)
            with patch.dict(os.environ, {"POSTGRES_PASSWORD": "overridden"}):
                with self.assertRaises(ValueError):
                    production.read_settings(path)
            link = Path(directory) / "link"
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                production.read_settings(link)

    def test_ambiguous_dotenv_syntax_rejected(self):
        for content in ("TOKEN=${OTHER}\n", "TOKEN='quoted'\n", "TOKEN=x # comment\n",
                        "TOKEN=one\nTOKEN=two\n", "not-an-assignment\n"):
            with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {}, clear=True):
                path = Path(directory) / ".env"
                path.write_text(content)
                path.chmod(0o600)
                with self.assertRaises(ValueError):
                    production.read_settings(path)


@unittest.skipUnless(os.environ.get("SANSCUE_COMPOSE_BIN"),
                     "set SANSCUE_COMPOSE_BIN to run actual Compose merge checks (no daemon needed)")
class ProductionCompose(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / ".env"
            write_fixture(path, fixture())
            env = {k: v for k, v in os.environ.items()
                   if k not in fixture() and not k.startswith("COMPOSE_")}
            result = subprocess.run(
                [os.environ["SANSCUE_COMPOSE_BIN"], "--project-name", "sanscue-production",
                 "--env-file", str(path), "-f", "compose.yaml", "-f", "compose.production.yaml",
                 "--profile", "*", "config", "--format", "json"],
                cwd=ROOT, env=env, text=True, capture_output=True, timeout=30,
            )
            if result.returncode:
                raise AssertionError("Compose configuration failed with synthetic settings; inspect configuration separately")
            cls.config = json.loads(result.stdout)

    def test_only_public_web_ports_and_private_bridge(self):
        services = self.config["services"]
        public = []
        for name, service in services.items():
            for port in service.get("ports", []):
                if port.get("host_ip") != "127.0.0.1":
                    public.append((name, port["published"]))
        self.assertCountEqual(public, [("web", "80"), ("web", "443")])
        self.assertEqual(services["sessions-db"].get("ports", []), [])
        for name in ("sessions", "topics-and-questions", "bee-db"):
            self.assertTrue(services[name]["ports"])
            self.assertTrue(all(p["host_ip"] == "127.0.0.1" for p in services[name]["ports"]))

    def test_https_origin_and_operator_required(self):
        env = self.config["services"]["sessions"]["environment"]
        self.assertEqual(env["SESSIONS_PUBLIC_ORIGIN"], "https://sanscue.acme.org")
        self.assertEqual(env["SESSIONS_ALLOW_INSECURE_LOOPBACK"], "false")
        self.assertEqual(env["SESSIONS_PUBLIC_RATE_LIMITS"], "true")
        self.assertEqual(env["SESSIONS_TRUST_CLIENT_IP_HEADER"], "true")
        for key in production.KEYS:
            self.assertEqual(env[key], fixture()[key])

    def test_application_roles_separate_from_bootstrap(self):
        for name, prefix, key in (("sessions-db", "POSTGRES", "SESSIONS_DB_BOOTSTRAP_PASSWORD"),
                                  ("bee-db", "BEE_POSTGRES", "BEE_DB_BOOTSTRAP_PASSWORD"),
                                  ("topics-db", "TOPICS_POSTGRES", "TOPICS_DB_BOOTSTRAP_PASSWORD")):
            env = self.config["services"][name]["environment"]
            self.assertEqual(env["POSTGRES_USER"], "postgres")
            self.assertEqual(env["POSTGRES_PASSWORD"], fixture()[key])
            self.assertEqual(env["SANSCUE_APP_PASSWORD"], fixture()[prefix + "_PASSWORD"])
            self.assertEqual(env["SANSCUE_APP_DB"], fixture()[prefix + "_DB"])
            self.assertTrue(any(v["target"] == "/docker-entrypoint-initdb.d/10-app.sh"
                                and v["read_only"] for v in self.config["services"][name]["volumes"]))
        for name in ("sessions", "topics-and-questions", "bee-connection", "web"):
            values = self.config["services"][name].get("environment", {}).values()
            self.assertTrue(all(fixture()[key] not in values for key in production.BOOTSTRAP))

    def test_worker_profile_restarts_and_persistent_volumes(self):
        services = self.config["services"]
        self.assertEqual(services["bee-connection"]["profiles"], ["container-worker"])
        self.assertTrue(all(s["restart"] == "unless-stopped" for s in services.values()))
        mounts = {v["target"]: v for v in services["web"]["volumes"]}
        self.assertTrue(mounts["/etc/caddy/Caddyfile"]["read_only"])
        self.assertTrue(mounts["/etc/caddy/Caddyfile"]["source"].endswith("Caddyfile.production"))
        for target in ("/data", "/config"):
            self.assertEqual(mounts[target]["type"], "volume")
        for name in ("sessions", "bee", "topics"):
            self.assertEqual(self.config["volumes"][name + "-db-data"]["name"],
                             "sanscue-production-" + name + "-db-data")


if __name__ == "__main__":
    unittest.main()
