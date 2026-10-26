"""Capture a real synthetic setup terminal without consuming its stdin."""

import fcntl
import json
import os
import pty
import select
import signal
import subprocess
import sys
import termios
import time


def main():
    binary, profile, tools, confirm = sys.argv[1:]
    environment = dict(os.environ, HOME=profile, PATH=tools + ":/usr/bin:/bin",
                       XDG_CONFIG_HOME=profile + "/.config", XDG_DATA_HOME=profile + "/.local/share")
    environment.pop("CLAUDE_CONFIG_DIR", None)
    environment.pop("CODEX_HOME", None)
    master, slave = pty.openpty()

    def session():
        os.setsid()
        fcntl.ioctl(1, termios.TIOCSCTTY, 0)

    process = subprocess.Popen([binary, "init"], env=environment, stdin=subprocess.DEVNULL,
                               stdout=slave, stderr=slave, preexec_fn=session)
    os.close(slave)
    prompts = [(b"Select hosts", b"codex,claude\n"), (b"Durable mailbox home", b"\n"),
               (b"Show dispatcher start instructions?", b"y\n"),
               (b"Apply this user setup?", confirm.encode() + b"\n")]
    output, answered = b"", 0
    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try:
                    output += os.read(master, 65536)
                except OSError:
                    break
                if answered < len(prompts) and prompts[answered][0] in output:
                    os.write(master, prompts[answered][1])
                    answered += 1
            if process.poll() is not None:
                break
        code = process.wait(timeout=5)
        print(json.dumps({"code": code, "answered": answered,
                          "transcript": output.decode(errors="replace").replace("\r", "")}))
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        os.close(master)


if __name__ == "__main__":
    main()
