"""Install the actual candidate binary and probe unattended argv under a tty."""

import fcntl
import hashlib
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
    binary, profile_arg, tools_arg, version = sys.argv[1:]
    profile, tools = Path(profile_arg), Path(tools_arg)
    root = profile.parent
    system, machine = platform.system(), platform.machine()
    target = {
        ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
        ("Linux", "aarch64"): "aarch64-unknown-linux-gnu",
        ("Darwin", "x86_64"): "x86_64-apple-darwin",
        ("Darwin", "arm64"): "aarch64-apple-darwin",
    }[(system, machine)]
    asset = root / f"merl-v{version}-{target}.tar.gz"
    with tarfile.open(asset, "w:gz") as archive:
        archive.add(binary, arcname="merl/merl")
    (root / "SHA256SUMS").write_text(f"{hashlib.sha256(asset.read_bytes()).hexdigest()}  {asset.name}\n")
    curl = tools / "curl"
    curl.write_text('''#!/usr/bin/env python3
import os, pathlib, shutil, sys
args = sys.argv[1:]
shutil.copyfile(pathlib.Path(os.environ["ASSETS"]) / args[-1].split("/")[-1], args[args.index("--output")+1])
''')
    curl.chmod(0o755)
    environment = dict(os.environ, HOME=str(profile), PATH=str(tools) + ":/usr/bin:/bin",
                       ASSETS=str(root), TMPDIR=str(root), XDG_CONFIG_HOME=str(profile / ".config"),
                       XDG_DATA_HOME=str(profile / ".local/share"))
    environment.pop("CLAUDE_CONFIG_DIR", None)
    environment.pop("CODEX_HOME", None)
    master, slave = pty.openpty()

    def terminal_session():
        os.setsid()
        fcntl.ioctl(1, termios.TIOCSCTTY, 0)

    installer = Path(__file__).resolve().parents[2] / "install.sh"
    destination = root / "verified bin"
    process = subprocess.Popen(["sh", "-s", "--", "--version", "v" + version,
                                "--bin-dir", str(destination), "--unattended", "--setup", "--",
                                "--host", "none", "--home", "--yes"],
                               env=environment, stdin=subprocess.PIPE, stdout=slave,
                               stderr=slave, preexec_fn=terminal_session)
    os.close(slave)
    process.stdin.write(installer.read_bytes())
    process.stdin.close()
    output = b""
    prompted = False
    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try:
                    output += os.read(master, 65536)
                except OSError:
                    break
                if b"Show dispatcher start instructions?" in output:
                    prompted = True
                    break
            if process.poll() is not None:
                break
        stalled = prompted
        if prompted:
            os.killpg(process.pid, signal.SIGKILL)
        try:
            code = process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            stalled = True
            os.killpg(process.pid, signal.SIGKILL)
            code = process.wait()
        print(json.dumps({"code": code, "prompted": prompted, "stalled": stalled,
                          "binary_installed": (destination / "merl").is_file(),
                          "transcript": output.decode(errors="replace")}))
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        os.close(master)


if __name__ == "__main__":
    main()
