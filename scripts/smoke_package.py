"""Install the native archive in a disposable user directory without network."""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--tag", required=True)
    args = parser.parse_args()
    installer = Path(__file__).resolve().parents[1] / "install.sh"
    with tempfile.TemporaryDirectory(prefix="merl clean user ") as temporary:
        root = Path(temporary)
        archive = root / args.archive.name
        shutil.copyfile(args.archive, archive)
        (root / "SHA256SUMS").write_text(f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
        tools = root / "tools"
        tools.mkdir()
        curl = tools / "curl"
        curl.write_text('''#!/bin/sh
set -eu
while [ "$#" -gt 0 ]; do
    if [ "$1" = --output ]; then output=$2; shift 2; else url=$1; shift; fi
done
cp "$SMOKE_ASSETS/${url##*/}" "$output"
''')
        curl.chmod(0o755)
        env = dict(os.environ, PATH=f"{tools}:{os.environ['PATH']}", SMOKE_ASSETS=str(root), TMPDIR=str(root))
        destination = root / "user bin"
        subprocess.run(["sh", str(installer), "--unattended", "--no-setup", "--version", args.tag, "--bin-dir", str(destination)],
                       env=env, check=True)
        version = subprocess.check_output([str(destination / "merl"), "--version"], text=True).strip()
        if version != f"merl {args.tag[1:]}":
            raise SystemExit("installed version does not match packaged release")
        subprocess.run([str(destination / "merl"), "help"], check=True)
        print("Native archive -> checksum-verified clean-directory install -> version/help: PASS")


if __name__ == "__main__":
    main()
