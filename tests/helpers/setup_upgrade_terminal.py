"""Install a real candidate into a synthetic profile through its own terminal."""

import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import pty
import select
import signal
import subprocess
import sys
import tarfile
import termios
import time


def main():
    binary, repository, profile, answer = sys.argv[1:]
    binary, repository, profile = Path(binary), Path(repository), Path(profile)
    fixtures = profile.parent / (profile.name + "-installer-fixtures")
    tools = fixtures / "tools"
    tools.mkdir(parents=True, exist_ok=True)
    version = subprocess.check_output([binary, "--version"], text=True).strip().split()[1]
    target = {("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
              ("Linux", "aarch64"): "aarch64-unknown-linux-gnu",
              ("Darwin", "x86_64"): "x86_64-apple-darwin",
              ("Darwin", "arm64"): "aarch64-apple-darwin"}[(platform.system(), platform.machine())]
    archive = fixtures / f"merl-v{version}-{target}.tar.gz"
    if not archive.exists():
        content = binary.read_bytes()
        with tarfile.open(archive, "w:gz") as tar:
            info = tarfile.TarInfo("merl/merl")
            info.mode, info.size = 0o755, len(content)
            tar.addfile(info, io.BytesIO(content))
    (fixtures / "SHA256SUMS").write_text(f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
    (tools / "curl").write_text('''#!/usr/bin/env python3
import os, pathlib, shutil, sys
args=sys.argv[1:]
shutil.copyfile(pathlib.Path(os.environ["SYNTHETIC_ASSETS"])/args[-1].rsplit("/",1)[-1],args[args.index("--output")+1])
''')
    for host in ["codex", "claude"]:
        (tools / host).write_text("#!/bin/sh\nexit 75\n")
    for tool in tools.iterdir():
        tool.chmod(0o755)
    env = dict(os.environ, HOME=str(profile), PATH=str(tools) + ":/usr/bin:/bin",
               XDG_CONFIG_HOME=str(profile / ".config"), XDG_DATA_HOME=str(profile / ".data"),
               SYNTHETIC_ASSETS=str(fixtures))
    for variable in ["CODEX_HOME", "CLAUDE_CONFIG_DIR"]:
        env.pop(variable, None)
    master, slave = pty.openpty()

    def terminal():
        os.setsid()
        fcntl.ioctl(1, termios.TIOCSCTTY, 0)

    process = subprocess.Popen(["sh", "-s", "--", "--version", "v" + version,
                                "--bin-dir", str(profile / "bin")], env=env,
                               stdin=subprocess.PIPE, stdout=slave, stderr=slave,
                               preexec_fn=terminal)
    os.close(slave)
    process.stdin.write((repository / "install.sh").read_bytes())
    process.stdin.close()
    output = b""
    seen = set()
    prompts = [(b"Run guided setup now?", b"n\n" if answer == "entry-no" else b"y\n"), (b"Select hosts", b"codex,claude\n"),
               (b"Durable mailbox home [", b"\n"),
               (b"Show dispatcher start instructions?", b"\n"),
               (b"Apply this user setup?", answer.encode() + b"\n")]
    try:
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try:
                    output += os.read(master, 65536)
                except OSError:
                    break
                for prompt, response in prompts:
                    if prompt in output and prompt not in seen:
                        os.write(master, response)
                        seen.add(prompt)
            if process.poll() is not None:
                break
        code = process.wait(timeout=2)
        while select.select([master], [], [], 0)[0]:
            try:
                output += os.read(master, 65536)
            except OSError:
                break
        print(json.dumps({"code": code, "transcript": output.decode(errors="replace").replace("\r", "")}))
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        os.close(master)


if __name__ == "__main__":
    main()
