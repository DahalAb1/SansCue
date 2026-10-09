"""Database-independent safety checks for the acceptance runner."""
import importlib.util
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("synthetic-mvp.py")
SPEC = importlib.util.spec_from_file_location("synthetic_mvp", MODULE_PATH)
synthetic = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(synthetic)


class AcceptanceRunnerSafetyTests(unittest.TestCase):
    def urls(self):
        return {
            "DATABASE_URL": "postgresql://u:p@127.0.0.1:5432/sessions_test",
            "BEE_DATABASE_URL": "postgresql://u:p@localhost:5432/bee_test",
            "TOPICS_DATABASE_URL": "postgresql://u:p@[::1]:5432/topics_test",
        }

    def test_accepts_three_distinct_loopback_databases(self):
        self.assertEqual(len(synthetic.database_targets(self.urls())), 3)

    def test_rejects_remote_database_host(self):
        env = self.urls()
        env["BEE_DATABASE_URL"] = "postgresql://u:p@db.example.test:5432/bee_test"
        with self.assertRaisesRegex(RuntimeError, "local loopback"):
            synthetic.database_targets(env)

    def test_rejects_connection_target_query_overrides(self):
        for option in ("host=remote.example", "hostaddr=192.0.2.1", "port=6432",
                       "dbname=other_db", "service=remote_service", "%68ost=remote.example"):
            with self.subTest(option=option):
                env = self.urls()
                env["BEE_DATABASE_URL"] += "?" + option
                with self.assertRaisesRegex(RuntimeError, "target overrides"):
                    synthetic.database_targets(env)

    def test_rejects_same_database_under_different_loopback_spellings(self):
        env = self.urls()
        env["BEE_DATABASE_URL"] = "postgresql://u:p@localhost:5432/sessions_test"
        with self.assertRaisesRegex(RuntimeError, "three distinct"):
            synthetic.database_targets(env)

    def test_all_http_services_are_pinned_to_loopback(self):
        base = self.urls() | {"HTTP_HOST": "0.0.0.0"}
        sessions, topics, _worker = synthetic.service_environments(
            base, "operator", "bee-token", "topics-token", "epoch", "hmac-key"
        )
        self.assertEqual(sessions["HTTP_HOST"], "127.0.0.1")
        self.assertEqual(topics["HTTP_HOST"], "127.0.0.1")


if __name__ == "__main__":
    unittest.main()
