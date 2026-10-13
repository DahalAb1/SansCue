#!/usr/bin/env python3
"""Opt-in real PostgreSQL 17 outage test; owns only its disposable container."""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[3]


def run(*args):
    return subprocess.check_output(args, text=True, timeout=120).strip()


def request(port, path, status, body):
    start = time.monotonic()
    try:
        response = urllib.request.urlopen(f"http://127.0.0.1:{port}/{path}", timeout=3)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        assert response.status == status, (path, response.status, status)
        assert response.headers.get_content_type() == "application/json"
        assert json.load(response) == {"status": body}
    assert time.monotonic() - start < 3, "health request exceeded timeout tolerance"


def wait_ready(port, child):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        assert child.poll() is None, "sessions exited during recovery"
        try:
            request(port, "readyz", 200, "ready")
            request(port, "healthz", 200, "ok")
            return
        except (OSError, AssertionError):
            time.sleep(0.2)
    raise AssertionError("readiness did not recover within 30 seconds")


def main():
    for tool in ("docker", "cargo"):
        if not shutil.which(tool):
            raise SystemExit(f"Missing prerequisite: {tool}; ask the environment Fixer, do not skip this check")
    run("docker", "info", "--format", "{{.ServerVersion}}")
    subprocess.run(["cargo", "build", "--locked", "--package", "sessions-service"], cwd=ROOT, check=True)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version=1"], cwd=ROOT, text=True))
    binary = Path(metadata["target_directory"]) / "debug/sessions-service"
    name = f"sanscue-outage-test-{uuid.uuid4().hex}"
    child = None
    # No host mounts or named volumes. Never stop an existing database.
    try:
        run("docker", "run", "--detach", "--name", name,
            "--publish", "127.0.0.1::5432", "--env", "PGDATA=/tmp/sanscue-test-data",
            "--env", "POSTGRES_DB=sessions", "--env", "POSTGRES_USER=sessions_app",
            "--env", "POSTGRES_PASSWORD=disposable-test-only", "postgres:17")
        db_port = run("docker", "port", name, "5432/tcp").rsplit(":", 1)[1]
        deadline = time.monotonic() + 60
        # The initialization server is Unix-socket-only; wait for final TCP service.
        while subprocess.run(["docker", "exec", name, "pg_isready", "-h", "127.0.0.1", "-U", "sessions_app",
                               "-d", "sessions"], stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, timeout=10).returncode:
            if time.monotonic() >= deadline:
                raise AssertionError("disposable PostgreSQL startup timed out; ask Fixer")
            time.sleep(0.2)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        environment = dict(os.environ,
            DATABASE_URL=f"postgres://sessions_app:disposable-test-only@127.0.0.1:{db_port}/sessions",
            HTTP_HOST="127.0.0.1", HTTP_PORT=str(port), RUST_LOG="info", NO_COLOR="1")
        environment.pop("PGOPTIONS", None)
        # The service rejects port zero. Bind failures remain fatal, never choose a fixed port.
        with tempfile.TemporaryFile(mode="w+") as logs:
            child = subprocess.Popen([binary], env=environment, stdout=logs, stderr=logs)
            wait_ready(port, child)
            pid = child.pid
            # SIGKILL closes real DB connections. Restart the same container, not sessions.
            run("docker", "kill", name)
            for _ in range(3):
                request(port, "healthz", 200, "ok")
                request(port, "readyz", 503, "not_ready")
                assert child.poll() is None
            run("docker", "start", name)
            wait_ready(port, child)
            assert child.pid == pid and child.poll() is None
            print("PASS: actual DB outage: liveness 200, readiness 503; recovery 200 with same sessions PID")
    finally:
        if child is not None and child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        subprocess.run(["docker", "rm", "--force", "--volumes", name], check=False,
                       stdout=subprocess.DEVNULL, timeout=30)


if __name__ == "__main__":
    main()
