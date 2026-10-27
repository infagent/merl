#!/usr/bin/env python3
"""Compare a complete synthetic exchange; never launch a real host or model."""

import argparse
import hashlib
import json
from pathlib import Path
import socket
import subprocess
import tempfile


class Exchange:
    def __init__(self, binary, root, inline):
        self.binary = str(binary.resolve())
        self.root = root
        self.home = root / "mail"
        self.inline = inline
        self.commands = []
        self.checkpoints = []
        self.host = root / "synthetic-host"
        self.host.write_text(
            "#!/usr/bin/env python3\nimport json,pathlib,sys\n"
            "p=pathlib.Path(__file__).parent\na=sys.argv[1:]\n"
            "assert a[0]=='queue'\n"
            "with (p/'accepted.jsonl').open('a') as f:\n"
            " f.write(json.dumps({'thread':a[a.index('--thread')+1],"
            "'notice':a[a.index('--message')+1]})+'\\n')\n"
        )
        self.host.chmod(0o700)

    def command(self, phase, *args):
        argv = [self.binary, "mail", "--home", str(self.home), *args, "--json"]
        return self.execute(phase, argv)

    def execute(self, phase, argv):
        result = subprocess.run(argv, capture_output=True, text=True, timeout=15)
        self.commands.append({"phase": phase, "argv": argv, "exit": result.returncode,
                              "stdout_bytes": len(result.stdout.encode()),
                              "stderr_bytes": len(result.stderr.encode()),
                              "stdout": result.stdout, "stderr": result.stderr})
        if result.returncode:
            raise RuntimeError(f"synthetic {phase} command failed: {result.stdout} {result.stderr}")
        return json.loads(result.stdout)

    def send(self, sender, recipient, message_id, body, reply=None):
        source = self.root / (message_id + ".txt")
        source.write_text(body)
        args = ["send", "--from", sender, "--to", recipient, "--id", message_id,
                "--kind", "reply" if reply else "assign", "--body-file", str(source)]
        if reply:
            args.extend(["--in-reply-to", reply])
        self.command("exchange", *args)

    def consume(self, agent, message_id, expected_body, event):
        if self.inline:
            notice = json.loads(event["notice"])
            assert notice["schema"] == "merl.notification/v1"
            assert notice["agent"] == agent
            message = next(item for item in notice["messages"] if item["id"] == message_id)
            assert message["body"] == expected_body
        else:
            headers = self.command("exchange", "list", "--agent", agent, "--pending")
            assert headers["messages"][0]["id"] == message_id
            message = self.command("exchange", "read", "--agent", agent, "--id", message_id)
            assert message["message"]["body"] == expected_body
        checkpoint = json.dumps({"agent": agent, "consumed": [message_id], "next": "finish assigned work"}) + "\n"
        (self.root / (agent + "-checkpoint.json")).write_text(checkpoint)
        self.checkpoints.append({"agent": agent, "bytes": len(checkpoint.encode())})
        if self.inline:
            self.command("exchange", "handle", "--agent", agent, "--ids", message_id, "--read", "--receipt")
        else:
            self.command("exchange", "receipt", "--agent", agent, "--id", message_id)

    def run(self):
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(self.root / "host.sock"))
        try:
            for index, agent in enumerate(("bob", "alice"), start=1):
                self.command("setup", "register", "--agent", agent)
                self.command("setup", "bind", "--agent", agent, "--session",
                             f"00000000-0000-4000-8000-{index:012}",
                             "--socket", str(self.root / "host.sock"), "--expected-generation", "0")
            self.execute("help", [self.binary, "usage", "--json"])
            self.send("alice", "bob", "assignment-1", "Synthetic request: report ready.\n")
            self.command("exchange", "dispatch", "--codex", str(self.host))
            events = [json.loads(line) for line in (self.root / "accepted.jsonl").read_text().splitlines()]
            self.consume("bob", "assignment-1", "Synthetic request: report ready.\n", events[-1])
            self.send("bob", "alice", "result-1", "Synthetic result: ready.\n", "assignment-1")
            if self.inline:
                self.command("exchange", "handle", "--agent", "bob", "--ids", "assignment-1",
                             "--complete", "--reply", "result-1")
            else:
                self.command("exchange", "complete", "--agent", "bob", "--id", "assignment-1", "--reply", "result-1")
            self.command("exchange", "dispatch", "--codex", str(self.host))
            events = [json.loads(line) for line in (self.root / "accepted.jsonl").read_text().splitlines()]
            self.consume("alice", "result-1", "Synthetic result: ready.\n", events[-1])
            for agent in ("bob", "alice"):
                status = self.command("verification", "status", "--agent", agent)
                assert (status["unread"], status["unreceipted"], status["outstanding"]) == (0, 0, 0)
            assert len(events) == 2
            version = subprocess.check_output([self.binary, "--version"], text=True).strip()
            return {
                "binary_sha256": hashlib.sha256(Path(self.binary).read_bytes()).hexdigest(), "version": version,
                "commands_by_phase": {phase: sum(row["phase"] == phase for row in self.commands)
                                      for phase in ("setup", "help", "exchange", "verification")},
                "stdout_bytes": sum(row["stdout_bytes"] for row in self.commands),
                "stderr_bytes": sum(row["stderr_bytes"] for row in self.commands),
                "help_stdout_bytes": sum(row["stdout_bytes"] for row in self.commands if row["phase"] == "help"),
                "notification_bytes": sum(len(event["notice"].encode()) for event in events),
                "host_accepted_events": len(events), "checkpoint_bytes": sum(row["bytes"] for row in self.checkpoints),
                "real_model_calls": 0, "provider_usage": None, "commands": self.commands, "events": events,
            }
        finally:
            listener.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path, help="released 0.5 binary")
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    results = {}
    for name, binary, inline in [("baseline", args.baseline, False), ("candidate", args.candidate, True)]:
        with tempfile.TemporaryDirectory(prefix="merl-exchange-") as temporary:
            results[name] = Exchange(binary, Path(temporary), inline).run()
    results["limitations"] = (
        "Same synthetic request/result, cold usage help, two checkpoints and final verification. "
        "Setup/help/verification shown separately; version/hash probes excluded from exchange counts. "
        "No real host or inference: CLI counts are not measured tool/model turns or token savings. "
        "All stdout, notice and checkpoint bytes are included; provider usage is unknown/not invoked."
    )
    args.output.write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps({name: {key: value for key, value in result.items() if key not in ("commands", "events")}
                      for name, result in results.items() if isinstance(result, dict)}, indent=2))


if __name__ == "__main__":
    main()
