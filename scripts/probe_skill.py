"""Capture skill injection on an installed Codex using a rejecting loopback mock.

This optional host check performs no inference. It stores only Merl's own skill
body and synthetic invocation, never the host's other prompt text or headers.
"""

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading


def probe(codex, skill):
    executable = shutil.which(codex)
    if executable is None:
        raise ValueError("Codex executable is unavailable; no fallback is attempted")
    text = skill.read_text()
    body = text.split("---", 2)[2].strip()
    invocation = "$merl project/member-a Synthetic starting guidance. Do not run tools."
    captured = []

    class Reject(BaseHTTPRequestHandler):
        def do_POST(self):
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= 1024 * 1024:
                self.send_error(413)
                return
            captured.append(self.rfile.read(length).decode())
            self.send_response(400)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"error":{"message":"synthetic rejection; no inference"}}')

        def log_message(self, *_args):
            pass

    with tempfile.TemporaryDirectory(prefix="merl skill probe ") as scratch:
        root = Path(scratch)
        profile = root / "profile"
        codex_home = profile / ".codex"
        destination = profile / ".agents/skills/merl/SKILL.md"
        destination.parent.mkdir(parents=True)
        destination.write_text(text)
        codex_home.mkdir()
        work = root / "work"
        work.mkdir()
        env = {"PATH": os.defpath, "HOME": str(profile), "CODEX_HOME": str(codex_home),
               "XDG_CONFIG_HOME": str(profile / ".config"),
               "XDG_DATA_HOME": str(profile / ".local/share"),
               "OTEL_SDK_DISABLED": "true", "NO_PROXY": "127.0.0.1,localhost"}
        version = subprocess.check_output([executable, "--version"], env=env, text=True).strip()
        server = HTTPServer(("127.0.0.1", 0), Reject)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        provider = {"name": "Merl rejecting loopback", "base_url": f"http://127.0.0.1:{server.server_port}/v1",
                    "wire_api": "responses", "requires_openai_auth": False,
                    "supports_websockets": False, "request_max_retries": 0, "stream_max_retries": 0}
        # CLI overrides use TOML, whose inline-table strings share JSON quoting.
        table = "{" + ",".join(f"{key}={json.dumps(value)}" for key, value in provider.items()) + "}"
        command = [executable, "--no-daemon", "exec", "--ignore-user-config", "--ignore-rules",
                   "--ephemeral", "--skip-git-repo-check", "--sandbox", "read-only", "--cd", str(work),
                   "--model", "merl-synthetic-probe", "-c", 'model_provider="merl_probe"',
                   "-c", "model_providers.merl_probe=" + table,
                   "-c", "memories.use_memories=false", invocation]
        try:
            result = subprocess.run(command, env=env, stdin=subprocess.DEVNULL,
                                    capture_output=True, timeout=30)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
        requests = [json.loads(raw) for raw in captured]
        encoded = json.dumps(requests, ensure_ascii=False)
        # Decode JSON before checking text; escaped Unicode is still supplied text.
        def strings(value):
            if isinstance(value, str):
                yield value
            elif isinstance(value, dict):
                for item in value.values():
                    yield from strings(item)
            elif isinstance(value, list):
                for item in value:
                    yield from strings(item)
        values = list(strings(requests))
        if len(captured) != 1 or result.returncode == 0 or not any(body in s for s in values) or not any(invocation in s for s in values):
            raise ValueError(f"host capture did not establish skill/argument loading (requests={len(captured)}, exit={result.returncode}, bytes={len(encoded)})")
        return {"host_version": version, "skill_sha256": hashlib.sha256(text.encode()).hexdigest(),
                "request_count": len(captured), "mock_status": 400, "host_exit": result.returncode,
                "skill_body": body, "invocation": invocation, "skill_loaded": True,
                "arguments_preserved": True, "inference": False, "semantic_execution_verified": False,
                "global_changes": False, "credentials_supplied": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex", default="codex")
    parser.add_argument("--skill", type=Path, default=Path(__file__).resolve().parents[1] / "skills/merl/SKILL.md")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = probe(args.codex, args.skill)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Captured {result['host_version']}: skill and supplied text; HTTP 400, no inference.")


if __name__ == "__main__":
    main()
