"""End-to-end HTTP contract checks, using only Python's standard library."""
import json
import os
import secrets
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request


def main():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    token = secrets.token_hex(32)
    env = dict(os.environ, SCPRUST_TOKEN=token, SCPRUST_THREADS="2",
               SCPRUST_BIND=f"127.0.0.1:{port}")
    process = subprocess.Popen([sys.argv[1]], env=env)
    base = f"http://127.0.0.1:{port}"

    def request(path, data=None, authorized=True):
        headers = {"Content-Type": "application/json"}
        if authorized:
            headers["Authorization"] = "Bearer " + token
        req = urllib.request.Request(base + path, data=data, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=5) as result:
                return result.status, result.read()
        except urllib.error.HTTPError as error:
            return error.code, error.read()

    try:
        deadline = time.monotonic() + 10
        while True:
            try:
                assert request("/health")[0] == 200
                break
            except urllib.error.URLError:
                if time.monotonic() >= deadline or process.poll() is not None:
                    raise
                time.sleep(0.05)
        batch = {
            "points": [{"id": 3, "position": [3, 4, 0]},
                       {"id": 1, "position": [0, 0, 0]},
                       {"id": 2, "position": [0, 0, 6]}],
            "queries": [{"center": [0, 0, 0], "radius": 5, "max_hits": 1},
                        {"center": [0, 0, 0], "radius": 0, "max_hits": 512}]
        }
        data = json.dumps(batch).encode()
        assert request("/v1/radius", data, False)[0] == 401
        status, body = request("/v1/radius", data)
        assert status == 200, (status, body)
        assert json.loads(body) == {"results": [
            {"ids": [1], "truncated": True}, {"ids": [1], "truncated": False}]}
        assert request("/v1/radius", b"not JSON")[0] == 400
        assert request("/v1/radius", b"x" * (2 * 1024 * 1024 + 1))[0] == 413
        batch["points"][1]["id"] = 3
        assert request("/v1/radius", json.dumps(batch).encode())[0] == 400
        batch["points"] = []
        status, body = request("/v1/radius", json.dumps(batch).encode())
        assert status == 200
        assert all(result["ids"] == [] for result in json.loads(body)["results"])
        batch["queries"] *= 65
        assert request("/v1/radius", json.dumps(batch).encode())[0] == 400
        # Stateful core: version CAS, reusable graph, and ordered parallel jobs.
        assert request("/v1/world", authorized=False)[0] == 401
        status, body = request("/v1/world")
        assert status == 200
        initial = json.loads(body)["version"]
        assert initial["revision"] == 0
        assert len(initial["epoch"]) == 32
        key1 = {"id": 1, "generation": 7}
        key2 = {"id": 2, "generation": 7}
        key3 = {"id": 3, "generation": 7}
        update = {"base": initial, "entities": [
            {"key": key1, "position": [0, 0, 0], "faction": 1, "alive": True},
            {"key": key2, "position": [10, 0, 0], "faction": 2, "alive": True},
            {"key": key3, "position": [2, 0, 0], "faction": 2, "alive": False}],
            "navigation": [
                {"id": 1, "position": [0, 0, 0], "edges": [{"to": 2, "cost": 1}, {"to": 3, "cost": 10}]},
                {"id": 2, "position": [1, 0, 0], "edges": [{"to": 3, "cost": 2}]},
                {"id": 3, "position": [2, 0, 0], "edges": []}]}
        status, body = request("/v1/world", json.dumps(update).encode())
        assert status == 200, (status, body)
        info = json.loads(body)
        version = info["version"]
        assert version["revision"] == 1 and info["entities"] == 3 and info["nodes"] == 3
        assert request("/v1/world", json.dumps(update).encode())[0] == 409
        agent = {"actor": key1, "perception_radius": 100, "attack_radius": 5,
                 "move_speed": 20, "target_factions": [2]}
        core_batch = {"version": version, "items": [agent]}
        status, body = request("/v1/think", json.dumps(core_batch).encode())
        assert status == 200, (status, body)
        assert json.loads(body) == {"version": version, "results": [{
            "actor": key1, "target": key2, "action": "chase",
            "desired_velocity": [20, 0, 0], "distance": 10}]}
        observer = {"version": version, "items": [{"actor": key1, "radius": 20, "max_entities": 1}]}
        status, body = request("/v1/interest", json.dumps(observer).encode())
        assert status == 200
        assert json.loads(body)["results"] == [{"actor": key1, "entities": [key3], "truncated": True}]
        routes = {"version": version, "items": [{"start": 1, "goal": 3, "max_visited": 3},
                                                {"start": 3, "goal": 1, "max_visited": 3}]}
        status, body = request("/v1/routes", json.dumps(routes).encode())
        assert status == 200
        paths = json.loads(body)["results"]
        assert paths[0]["nodes"] == [1, 2, 3] and paths[0]["cost"] == 3
        assert paths[1]["status"] == "unreachable"
        invalid = dict(update, base=version)
        invalid["entities"] = update["entities"] + [update["entities"][0]]
        assert request("/v1/world", json.dumps(invalid).encode())[0] == 400
        assert json.loads(request("/v1/world")[1])["version"] == version
        core_batch["items"] *= 129
        assert request("/v1/think", json.dumps(core_batch).encode())[0] == 400
        # Omitting navigation retains it; explicitly [] clears it.
        status, body = request("/v1/world", json.dumps({"base": version, "entities": []}).encode())
        assert status == 200
        info = json.loads(body)
        assert info["nodes"] == 3 and info["entities"] == 0
        assert request("/v1/routes", json.dumps(routes).encode())[0] == 409
        assert request("/v1/think", json.dumps({"version": info["version"], "items": [agent]}).encode())[0] == 400
        status, body = request("/v1/world", json.dumps({"base": info["version"], "entities": [], "navigation": []}).encode())
        assert status == 200 and json.loads(body)["nodes"] == 0
        print("HTTP smoke tests passed")
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


if __name__ == "__main__":
    main()
