"""Package one frozen source revision; publish only a complete verified draft.

GitHub writes occur only in the explicit publish command. Source/version checks,
archive creation and assembly are also usable without credentials or a service.
"""

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

TARGETS = (
    "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin", "aarch64-apple-darwin",
)
MEMBERS = {"merl/merl", "merl/VERSION", "merl/REVISION", "merl/LICENSE",
           "merl/skills/merl-mail/SKILL.md"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def source(tag, sha, require_tag=False):
    require(re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag), "expected a stable vMAJOR.MINOR.PATCH tag")
    require(re.fullmatch(r"[0-9a-f]{40}", sha), "expected an exact 40-character source revision")
    require(git("rev-parse", "HEAD") == sha, "checkout differs from frozen release revision")
    if require_tag:
        require(git("rev-parse", f"refs/tags/{tag}^{{commit}}") == sha, "tag differs from frozen release revision")
    version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    require(tag == f"v{version}", "Cargo package version differs from release tag")
    require(json.loads(Path(".release-please-manifest.json").read_text())["."] == version,
            "Release Please manifest differs from package version")
    require(not git("status", "--porcelain=v1", "--untracked-files=no"), "source has uncommitted tracked changes")


def archive_name(tag, target):
    return f"merl-{tag}-{target}.tar.gz"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def package(args):
    source(args.tag, args.sha, args.require_tag)
    binary = Path(args.binary).resolve()
    reported = subprocess.check_output([str(binary), "--version"], text=True).strip()
    require(reported == f"merl {args.tag[1:]}", "built binary version differs from tag")
    subprocess.run([str(binary), "help"], check=True, stdout=subprocess.DEVNULL)
    content = {
        "merl/merl": binary.read_bytes(), "merl/VERSION": (args.tag[1:] + "\n").encode(),
        "merl/REVISION": (args.sha + "\n").encode(), "merl/LICENSE": Path("LICENSE").read_bytes(),
        "merl/skills/merl-mail/SKILL.md": Path("skills/merl-mail/SKILL.md").read_bytes(),
    }
    directory = Path(args.output)
    directory.mkdir(parents=True, exist_ok=True)
    output = directory / archive_name(args.tag, args.target)
    # Fixed headers make retrying archive assembly independent of build paths/time.
    with output.open("wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            for name, data in sorted(content.items()):
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mode = 0o755 if name == "merl/merl" else 0o644
                archive.addfile(info, io.BytesIO(data))
    print(output)


def check_archive(path, tag, sha):
    with tarfile.open(path) as archive:
        entries = archive.getmembers()
        require(len(entries) == len(MEMBERS) and {m.name for m in entries} == MEMBERS,
                f"unexpected archive members: {path.name}")
        require(all(m.isfile() and 0 < m.size <= 128 * 1024 * 1024 for m in entries),
                f"invalid archive members: {path.name}")
        for name, value in [("VERSION", tag[1:]), ("REVISION", sha)]:
            member = archive.getmember(f"merl/{name}")
            require(member.size <= 128, "oversized archive metadata")
            require(archive.extractfile(member).read().decode() == value + "\n",
                    f"archive {name} differs from release: {path.name}")
        for name, local in [("merl/LICENSE", "LICENSE"), ("merl/skills/merl-mail/SKILL.md", "skills/merl-mail/SKILL.md")]:
            require(archive.extractfile(name).read() == Path(local).read_bytes(),
                    f"archive {name} differs from frozen source")


def payload_names(tag):
    return {archive_name(tag, target) for target in TARGETS} | {"install.sh"}


def verify_assets(directory, tag, sha):
    expected = payload_names(tag)
    require({p.name for p in directory.iterdir()} == expected | {"SHA256SUMS"},
            "release requires exactly four platform archives, installer and checksums")
    lines = (directory / "SHA256SUMS").read_text().splitlines()
    entries = [line.split("  ") for line in lines]
    require(all(len(e) == 2 and re.fullmatch(r"[0-9a-f]{64}", e[0]) for e in entries),
            "invalid checksum manifest")
    require(len(entries) == len(expected) and {e[1] for e in entries} == expected,
            "checksum manifest does not cover the exact asset set")
    for checksum, name in entries:
        require(digest(directory / name) == checksum, f"checksum mismatch: {name}")
    for target in TARGETS:
        check_archive(directory / archive_name(tag, target), tag, sha)
    require((directory / "install.sh").read_bytes() == installer(tag),
            "installer differs from frozen source/tag")


def installer(tag):
    original = Path("install.sh").read_bytes()
    require(original.count(b"DEFAULT_VERSION=latest\n") == 1, "missing installer version marker")
    return original.replace(b"DEFAULT_VERSION=latest\n", f"DEFAULT_VERSION={tag}\n".encode())


def assemble(args):
    source(args.tag, args.sha)
    artifacts = Path(args.artifacts)
    expected = {archive_name(args.tag, target) for target in TARGETS}
    require({p.name for p in artifacts.iterdir()} == expected,
            "assembly requires the exact four-platform archive set")
    for name in sorted(expected):
        check_archive(artifacts / name, args.tag, args.sha)
    output = Path(args.output)
    require(not output.exists() or not list(output.iterdir()), "assembly output must be empty")
    output.mkdir(parents=True, exist_ok=True)
    for name in sorted(expected):
        shutil.copyfile(artifacts / name, output / name)
    (output / "install.sh").write_bytes(installer(args.tag))
    (output / "SHA256SUMS").write_text("".join(
        f"{digest(output / name)}  {name}\n" for name in sorted(payload_names(args.tag))))
    verify_assets(output, args.tag, args.sha)


def github(*args):
    return json.loads(subprocess.check_output(["gh", "api", *args], text=True))


def draft(tag, sha):
    repo = os.environ["GITHUB_REPOSITORY"]
    require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo), "invalid GitHub repository")
    release = github(f"repos/{repo}/releases/tags/{tag}")
    require(release["draft"] is True, "refusing to mutate a published release; retry only existing drafts")
    require(release["tag_name"] == tag and release["target_commitish"] == sha and not release["prerelease"],
            "draft differs from frozen stable tag/revision")
    obj = github(f"repos/{repo}/git/ref/tags/{tag}")["object"]
    for _ in range(4):
        if obj["type"] != "tag":
            break
        obj = github(f"repos/{repo}/git/tags/{obj['sha']}")["object"]
    require(obj["type"] == "commit" and obj["sha"] == sha, "remote tag differs from frozen revision")
    return release


def publish(args):
    source(args.tag, args.sha, require_tag=True)
    assets = Path(args.assets).resolve()
    verify_assets(assets, args.tag, args.sha)
    release = draft(args.tag, args.sha)
    expected = payload_names(args.tag) | {"SHA256SUMS"}
    require({a["name"] for a in release["assets"]} <= expected,
            "draft contains unexpected assets; inspect it before retrying")
    repo = os.environ["GITHUB_REPOSITORY"]
    subprocess.run(["gh", "release", "upload", args.tag, *[str(assets / n) for n in sorted(expected)],
                    "--repo", repo, "--clobber"], check=True)
    # Recheck remote names and bytes, not just successful upload process exits.
    uploaded = draft(args.tag, args.sha)
    require(len(uploaded["assets"]) == len(expected) and {a["name"] for a in uploaded["assets"]} == expected,
            "remote release asset set is incomplete")
    with tempfile.TemporaryDirectory(prefix="merl-release-verify-") as temporary:
        subprocess.run(["gh", "release", "download", args.tag, "--repo", repo, "--dir", temporary], check=True)
        verify_assets(Path(temporary), args.tag, args.sha)
        require(all((assets / n).read_bytes() == (Path(temporary) / n).read_bytes() for n in expected),
                "remote release bytes differ from built assets")
    final = draft(args.tag, args.sha)
    require(final["id"] == release["id"], "draft identity changed during publication")
    github(f"repos/{repo}/releases/{release['id']}", "--method", "PATCH", "-F", "draft=false", "-f", "make_latest=legacy")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ["check", "package", "assemble", "publish"]:
        command = commands.add_parser(name)
        command.add_argument("--tag", required=True)
        command.add_argument("--sha", required=True)
        if name in {"check", "package"}:
            command.add_argument("--require-tag", action="store_true")
        if name == "check":
            command.add_argument("--draft", action="store_true")
        elif name == "package":
            command.add_argument("--target", required=True, choices=TARGETS)
            command.add_argument("--binary", required=True)
            command.add_argument("--output", required=True)
        elif name == "assemble":
            command.add_argument("--artifacts", required=True)
            command.add_argument("--output", required=True)
        else:
            command.add_argument("--assets", required=True)
    args = parser.parse_args()
    try:
        if args.command == "check":
            source(args.tag, args.sha, args.require_tag)
            if args.draft:
                draft(args.tag, args.sha)
        else:
            {"package": package, "assemble": assemble, "publish": publish}[args.command](args)
    except (ValueError, OSError, subprocess.CalledProcessError, KeyError, tarfile.TarError) as error:
        parser.exit(1, f"merl-release: {error}\n")


if __name__ == "__main__":
    main()
