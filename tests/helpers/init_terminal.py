"""Drive only the synthetic CLI child's controlling terminal, never script stdin."""

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
    binary, profile, tools = sys.argv[1:]
    environment = dict(os.environ, HOME=profile, XDG_CONFIG_HOME=profile + "/.config",
                       XDG_DATA_HOME=profile + "/.local/share", PATH=tools + ":/usr/bin:/bin")
    environment.pop("CLAUDE_CONFIG_DIR", None)
    environment.pop("CODEX_HOME", None)
    master, slave = pty.openpty()

    def terminal_session():
        os.setsid()
        fcntl.ioctl(1, termios.TIOCSCTTY, 0)

    process = subprocess.Popen([binary, "init"], env=environment, stdin=subprocess.PIPE,
                               stdout=slave, stderr=slave, preexec_fn=terminal_session)
    os.close(slave)
    # If init mistakenly reads stdin, it selects codex instead of our tty answer.
    process.stdin.write(b"codex\n")
    process.stdin.close()
    output = b""
    prompts = [(b"Select hosts", b"claude\n"), (b"Durable mailbox home", b"\n"),
               (b"Show dispatcher start instructions?", b"n\n"), (b"Apply this user setup?", b"y\n")]
    answered = 0
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
        code = process.wait(timeout=2)
        print(json.dumps({"success": code == 0, "answered": answered,
                          "transcript": output.decode(errors="replace")}))
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        os.close(master)


if __name__ == "__main__":
    main()
