#!/usr/bin/env python3
"""Run one synthetic Bee-to-room acceptance pass against three native PostgreSQL DBs.

No Docker, Bee device, model provider, or AWS account is used. The supplied DBs
must be disposable/isolated; the script does not drop schemas or databases.
"""
import hashlib
import http.cookiejar
import ipaddress
import json
import os
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../.."))
ORIGIN = "http://127.0.0.1:5173"
SESSIONS = "http://127.0.0.1:3300"
INTERNAL = "http://127.0.0.1:3301"
TOPICS = "http://127.0.0.1:3302"
PROCS = []


def database_targets(env):
    names = []
    for key in ("DATABASE_URL", "BEE_DATABASE_URL", "TOPICS_DATABASE_URL"):
        value = env.get(key)
        if not value:
            raise RuntimeError(f"{key} is required")
        parsed = urllib.parse.urlparse(value)
        host = (parsed.hostname or "").lower()
        try:
            loopback = host == "localhost" or ipaddress.ip_address(host).is_loopback
        except ValueError:
            loopback = False
        if not loopback:
            raise RuntimeError(f"{key} must target a local loopback PostgreSQL host")
        overrides = {
            option.lower()
            for option, _value in urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
        } & {"host", "hostaddr", "port", "dbname", "service"}
        if overrides:
            raise RuntimeError(f"{key} must use URL authority fields; connection target overrides are not allowed")
        names.append(urllib.parse.unquote(parsed.path).lstrip("/"))
    if len(set(names)) != 3:
        raise RuntimeError("DATABASE_URL, BEE_DATABASE_URL and TOPICS_DATABASE_URL must name three distinct local databases")
    return names


def service_environments(base, credential, bee_token, topics_token, epoch, key):
    sessions_env = base | {
        "DATABASE_URL": base["DATABASE_URL"], "HTTP_HOST": "127.0.0.1", "HTTP_PORT": "3300",
        "SESSIONS_INTERNAL_HOST": "127.0.0.1", "SESSIONS_INTERNAL_PORT": "3301",
        "SESSIONS_PUBLIC_ORIGIN": ORIGIN, "SESSIONS_ALLOW_INSECURE_LOOPBACK": "true",
        "SESSIONS_OPERATOR_TOKEN_SHA256": hashlib.sha256(credential.encode()).hexdigest(),
        "SESSIONS_OPERATOR_EPOCH": epoch, "SESSIONS_IDEMPOTENCY_HMAC_KEY": key,
        "SESSIONS_BEE_SERVICE_TOKEN": bee_token,
        "SESSIONS_TOPICS_SERVICE_TOKEN": topics_token,
    }
    topics_env = base | {"HTTP_HOST": "127.0.0.1", "HTTP_PORT": "3302",
                        "TOPICS_DATABASE_URL": base["TOPICS_DATABASE_URL"],
                        "TOPICS_BEE_SERVICE_TOKEN": topics_token,
                        "SESSIONS_INTERNAL_URL": INTERNAL,
                        "SESSIONS_TOPICS_SERVICE_TOKEN": topics_token}
    worker_env = base | {"BEE_DATABASE_URL": base["BEE_DATABASE_URL"],
                         "SESSIONS_INTERNAL_URL": INTERNAL,
                         "SESSIONS_BEE_SERVICE_TOKEN": bee_token,
                         "TOPICS_INTERNAL_URL": TOPICS,
                         "TOPICS_BEE_SERVICE_TOKEN": topics_token}
    return sessions_env, topics_env, worker_env


def start(command, env):
    PROCS.append(subprocess.Popen(command, cwd=ROOT, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))


def request(client, method, path, body=None, headers=None, expected=(200,)):
    data = None if body is None else json.dumps(body).encode()
    request_headers = {"Origin": ORIGIN, "Accept": "application/json"}
    if data is not None:
        request_headers["Content-Type"] = "application/json"
    request_headers.update(headers or {})
    req = urllib.request.Request(path if path.startswith("http") else SESSIONS + path,
                                 data=data, headers=request_headers, method=method)
    try:
        with client.open(req, timeout=10) as response:
            raw = response.read()
            status = response.status
    except urllib.error.HTTPError as error:
        raw, status = error.read(), error.code
    if status not in expected:
        raise RuntimeError(f"{method} {path}: expected {expected}, got {status}: {raw[:300]!r}")
    if not raw:
        return {}
    return json.loads(raw)


def client_boot():
    jar = http.cookiejar.CookieJar()
    client = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    state = request(client, "GET", "/bootstrap")
    return client, state["csrf_token"]


def mutate(client, csrf, method, path, body, expected=(200,)):
    return request(client, method, path, body, {
        "X-CSRF-Token": csrf,
        "Idempotency-Key": str(uuid.uuid4()),
    }, expected)


def wait_ready():
    for _ in range(900):
        if any(p.poll() is not None for p in PROCS):
            raise RuntimeError("a service exited during startup; inspect its configuration locally")
        try:
            with urllib.request.urlopen(SESSIONS + "/readyz", timeout=1) as response:
                if response.status == 200:
                    return
        except Exception:
            pass
        time.sleep(.2)
    raise RuntimeError("sessions service did not become ready")


def wait_topics_ready():
    for _ in range(900):
        if any(p.poll() is not None for p in PROCS):
            raise RuntimeError("a service exited during startup; inspect its configuration locally")
        try:
            with urllib.request.urlopen(TOPICS + "/readyz", timeout=1) as response:
                if response.status == 200:
                    return
        except Exception:
            pass
        time.sleep(.2)
    raise RuntimeError("Topics service did not become ready")


def wait_until(predicate, description):
    for _ in range(80):
        value = predicate()
        if value:
            return value
        time.sleep(.25)
    raise RuntimeError(f"timed out waiting for {description}")


def replay_counts(output):
    counts = {}
    for token in output.split():
        if "=" in token:
            name, value = token.split("=", 1)
            if name in {"observations", "events", "gap_intervals"}:
                try:
                    counts[name] = int(value)
                except ValueError as error:
                    raise RuntimeError("replay CLI returned an invalid summary count") from error
    if set(counts) != {"observations", "events", "gap_intervals"}:
        raise RuntimeError("replay CLI returned an incomplete summary")
    return counts


def main():
    database_targets(os.environ)
    credential = secrets.token_hex(32)
    bee_token = secrets.token_urlsafe(32)
    topics_token = secrets.token_urlsafe(32)
    epoch = secrets.token_hex(16)
    key = secrets.token_hex(32)
    base = os.environ.copy()
    sessions_env, topics_env, worker_env = service_environments(
        base, credential, bee_token, topics_token, epoch, key
    )
    try:
        # Compile before launching: this avoids three concurrent Cargo lock
        # contenders and makes service readiness timing predictable.
        subprocess.run(["cargo", "build", "--quiet", "--target-dir", "target",
                        "-p", "sessions-service",
                        "-p", "topics-and-questions", "-p", "bee-connection"],
                       cwd=ROOT, env=base, check=True)
        start([os.path.join(ROOT, "target/debug/sessions-service")], sessions_env)
        start([os.path.join(ROOT, "target/debug/topics-and-questions")], topics_env)
        wait_ready()
        wait_topics_ready()

        speaker, speaker_csrf = client_boot()
        audience, audience_csrf = client_boot()
        other_audience, other_csrf = client_boot()
        mutate(speaker, speaker_csrf, "POST", "/operator/login", {"credential": credential})
        created = mutate(speaker, speaker_csrf, "POST", "/rooms", {"title": "Synthetic acceptance"}, (201,))
        room = created["state"]["room"]["id"]
        assert created["state"]["dashboard"]["percentages"]["clear"] is None
        join = created["join_url"]
        joined = mutate(audience, audience_csrf, "POST", join, {})
        assert joined["state"]["membership"]["role"] == "audience"
        other_joined = mutate(other_audience, other_csrf, "POST", join, {})
        assert other_joined["state"]["membership"]["role"] == "audience"

        conversation = str(uuid.uuid4())
        bind = mutate(speaker, speaker_csrf, "POST", f"/rooms/{room}/bee/bind",
                      {"conversation_id": conversation, "source_conversation_id": None})

        db = urllib.parse.urlparse(os.environ["DATABASE_URL"])
        psql_env = os.environ.copy() | {
            "PGHOST": db.hostname or "127.0.0.1", "PGPORT": str(db.port or 5432),
            "PGDATABASE": db.path.lstrip("/"), "PGUSER": urllib.parse.unquote(db.username or ""),
            "PGPASSWORD": urllib.parse.unquote(db.password or ""),
        }
        for option, value in urllib.parse.parse_qsl(db.query):
            if option in {"sslmode", "sslrootcert", "sslcert", "sslkey", "connect_timeout"}:
                psql_env["PG" + option.upper()] = value
        session_id = subprocess.run(
            ["psql", "-XAt", "-c",
             "SELECT session_id FROM bee_room_bindings WHERE room_id='" + room + "'"],
            cwd=ROOT, env=psql_env, check=True, capture_output=True, text=True,
        ).stdout.strip()
        if not session_id:
            raise RuntimeError("could not read the binding's speaker session from the sessions database")
        replay = ["cargo", "run", "--quiet", "--target-dir", "target",
                  "-p", "bee-connection", "--",
                  "services/bee-connection/fixtures/synthetic-recovery.json", conversation, room,
                  session_id]
        run_env = base | {"BEE_DATABASE_URL": base["BEE_DATABASE_URL"]}
        first_replay = subprocess.run(replay, cwd=ROOT, env=run_env, check=True,
                                      capture_output=True, text=True).stdout.strip()
        # Rerun the same fixture/binding: stable source IDs must not create extra candidates.
        second_replay = subprocess.run(replay, cwd=ROOT, env=run_env, check=True,
                                       capture_output=True, text=True).stdout.strip()
        first_counts = replay_counts(first_replay)
        second_counts = replay_counts(second_replay)
        assert first_counts["observations"] == 7 and first_counts["events"] == 3
        assert first_counts["gap_intervals"] > 0
        assert second_counts["observations"] == 14 and second_counts["events"] == 3, (
            "second replay must preserve raw arrivals while deduplicating canonical events"
        )
        # Replay establishes the fixture's verified capability flags before the
        # command worker creates/updates the sessions-authoritative binding.
        start([os.path.join(ROOT, "target/debug/bee-connection"), "--worker"], worker_env)
        wait_until(lambda: (lambda s: s["bee"]["status"] == "bound" and s)(
            request(speaker, "GET", f"/rooms/{room}/state")), "Bee binding command")

        def candidate_state():
            state = request(speaker, "GET", f"/rooms/{room}/state")
            candidates = state.get("question_candidates") or []
            return (state, candidates[0]) if len(candidates) == 3 else None

        speaker_state, candidate = wait_until(candidate_state, "candidate delivery")
        assert candidate["generator_version"] == "stub-v1"
        assert candidate["published"] is False
        assert candidate["evidence"]["excerpt"] in candidate["text"]
        candidate = speaker_state["question_candidates"][0]

        # Candidate/evidence are staff-only; audience sees neither before publication.
        assert request(audience, "GET", f"/rooms/{room}/state")["question_candidates"] is None
        request(audience, "POST", f"/rooms/{room}/questions/{candidate['candidate_id']}/publish", {}, {
            "X-CSRF-Token": audience_csrf, "Idempotency-Key": str(uuid.uuid4()),
        }, (403,))
        published = mutate(speaker, speaker_csrf, "POST",
                           f"/rooms/{room}/questions/{candidate['candidate_id']}/publish", {})
        question_id = published["question_id"]
        audience_state = request(audience, "GET", f"/rooms/{room}/state")
        assert audience_state["active_question"]["id"] == question_id
        assert "evidence" not in audience_state["active_question"]

        response_path = f"/rooms/{room}/questions/{question_id}/response"
        response_key = str(uuid.uuid4())
        response_headers = {"X-CSRF-Token": audience_csrf, "Idempotency-Key": response_key}
        response_body = {"response": "clear"}
        request(audience, "POST", response_path, response_body, response_headers)
        request(audience, "POST", response_path, response_body, response_headers)
        mutate(audience, audience_csrf, "POST", f"/rooms/{room}/qa",
               {"body": "Synthetic audience question", "question_id": question_id})
        audience_qa = request(audience, "GET", f"/rooms/{room}/qa")
        other_qa = request(other_audience, "GET", f"/rooms/{room}/qa")
        assert len(audience_qa["items"]) == 1
        assert other_qa["items"] == [], "another audience member must not see private written Q&A"
        qa = request(speaker, "GET", f"/rooms/{room}/qa")
        assert any(item["body"] == "Synthetic audience question" for item in qa["items"])
        final = request(speaker, "GET", f"/rooms/{room}/state")
        assert final["dashboard"]["respondents"] == 1
        assert final["dashboard"]["counts"]["clear"] == 1
        assert final["dashboard"]["percentages"]["clear"] == 100.0
        assert request(audience, "GET", f"/rooms/{room}/state")["dashboard"] is None
    finally:
        for proc in reversed(PROCS):
            if proc.poll() is None:
                proc.terminate()
        for proc in reversed(PROCS):
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
        if any(proc.poll() is None for proc in PROCS):
            raise RuntimeError("cleanup failed: a directly started service process is still running")
    print("PASS: synthetic replay -> idempotent event/candidate delivery -> staff-only preview -> publication -> audience rating/Q&A -> staff counts and audience privacy")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
