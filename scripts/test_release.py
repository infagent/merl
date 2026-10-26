"""Release CLI contracts with local Git tags and a synthetic GitHub boundary."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

from test_install import TARGETS

SCRIPT = Path(__file__).with_name("release.py")
INSTALLER = Path(__file__).resolve().parents[1] / "install.sh"


class Release(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="merl release ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = dict(os.environ)
        self.git("init", "-q")
        (self.root / "Cargo.toml").write_text('[package]\nversion="0.1.0"\n')
        (self.root / ".release-please-manifest.json").write_text('{".":"0.1.0"}')
        (self.root / "LICENSE").write_text("synthetic license")
        skill = self.root / "skills/merl-mail/SKILL.md"
        skill.parent.mkdir(parents=True)
        skill.write_text("synthetic skill")
        shutil.copyfile(INSTALLER, self.root / "install.sh")
        self.git("add", ".")
        self.git("-c", "user.name=Installer Test", "-c", "user.email=synthetic@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "-qm", "synthetic source")
        self.sha = self.git("rev-parse", "HEAD").strip()
        self.git("-c", "tag.gpgsign=false", "tag", "v0.1.0")
        self.binary = self.root / "built-merl"
        self.binary.write_text('#!/bin/sh\ncase "$1" in --version) echo "merl 0.1.0";; help) echo "merl mail";; esac\n')
        self.binary.chmod(0o755)

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True, stderr=subprocess.PIPE)

    def release(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), *args], cwd=self.root,
                              env=self.env, capture_output=True, text=True)

    def package(self, target="x86_64-unknown-linux-gnu", tag="v0.1.0", sha=None):
        return self.release("package", "--tag", tag, "--sha", sha or self.sha,
                            "--require-tag", "--target", target, "--binary", str(self.binary),
                            "--output", str(self.root / "packages"))

    def assembled_assets(self):
        for target in TARGETS.values():
            result = self.package(target)
            self.assertEqual(result.returncode, 0, result.stderr)
        result = self.release("assemble", "--tag", "v0.1.0", "--sha", self.sha,
                              "--artifacts", str(self.root / "packages"),
                              "--output", str(self.root / "assets"))
        self.assertEqual(result.returncode, 0, result.stderr)
        return self.root / "assets"

    def test_archives_bind_version_revision_skill_and_license_and_are_repeatable(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        archive = next((self.root / "packages").glob("*.tar.gz"))
        first = archive.read_bytes()
        with tarfile.open(archive) as contents:
            self.assertEqual(contents.extractfile("merl/VERSION").read(), b"0.1.0\n")
            self.assertEqual(contents.extractfile("merl/REVISION").read(), (self.sha + "\n").encode())
            self.assertEqual(contents.extractfile("merl/skills/merl-mail/SKILL.md").read(), b"synthetic skill")
            self.assertEqual(contents.extractfile("merl/LICENSE").read(), b"synthetic license")
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(archive.read_bytes(), first)

    def test_wrong_tags_revisions_and_dirty_sources_are_rejected(self):
        for tag, sha in [("main", self.sha), ("v9.9.9", self.sha), ("v0.1.0", "a" * 40)]:
            with self.subTest(tag=tag, sha=sha):
                self.assertNotEqual(self.package(tag=tag, sha=sha).returncode, 0)
        (self.root / "LICENSE").write_text("uncommitted change")
        self.assertNotEqual(self.package().returncode, 0)
        self.assertFalse((self.root / "packages").exists())

    def test_incomplete_asset_sets_cannot_be_assembled(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.release("assemble", "--tag", "v0.1.0", "--sha", self.sha,
                              "--artifacts", str(self.root / "packages"), "--output", str(self.root / "assets"))
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "assets/SHA256SUMS").exists())

    def mock_github(self, *, draft=True, drop=False, wrong_sha=False):
        state = {"id": 42, "tag_name": "v0.1.0", "target_commitish": self.sha,
                 "draft": draft, "prerelease": False, "assets": [],
                 "tag_sha": "a" * 40 if wrong_sha else self.sha, "drop": drop}
        self.state = self.root / "github.json"
        self.state.write_text(json.dumps(state))
        tools = self.root / "tools"
        tools.mkdir(exist_ok=True)
        gh = tools / "gh"
        gh.write_text('''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
a=sys.argv[1:]; p=pathlib.Path(os.environ["MOCK_GITHUB"]); s=json.loads(p.read_text()); remote=p.parent/"remote"; remote.mkdir(exist_ok=True)
if a[0]=="api":
    if "git/ref/tags/" in a[1]: print(json.dumps({"object":{"type":"commit","sha":s["tag_sha"]}}))
    elif "/releases/tags/" in a[1] and s["draft"]:
        print("HTTP 404: tag endpoint exposes published releases only", file=sys.stderr); sys.exit(1)
    elif "/releases?per_page=100&page=" in a[1]:
        print(json.dumps([{"id":i+100,"tag_name":"v0.0."+str(i),"draft":False} for i in range(100)] if a[1].endswith("page=1") else [s]))
    elif "--method" in a:
        s["draft"]=False; print(json.dumps(s))
    else: print(json.dumps(s))
elif a[:2]==["release","upload"]:
    for arg in a[3:]:
        f=pathlib.Path(arg)
        if f.is_file() and not (s["drop"] and f.name=="install.sh"): shutil.copyfile(f,remote/f.name)
    s["assets"]=[{"name":f.name} for f in sorted(remote.iterdir())]
elif a[:2]==["release","download"]:
    dst=pathlib.Path(a[a.index("--dir")+1])
    for f in remote.iterdir(): shutil.copyfile(f,dst/f.name)
else: sys.exit(2)
p.write_text(json.dumps(s))
''')
        gh.chmod(0o755)
        self.env.update(PATH=f"{tools}:{os.environ['PATH']}", MOCK_GITHUB=str(self.state),
                        GITHUB_REPOSITORY="synthetic/merl")

    def publish(self, assets):
        return self.release("publish", "--tag", "v0.1.0", "--sha", self.sha, "--assets", str(assets))

    def test_existing_drafts_are_found_after_the_first_release_page_without_the_published_tag_endpoint(self):
        self.mock_github()
        result = self.release("check", "--tag", "v0.1.0", "--sha", self.sha, "--require-tag", "--draft")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(self.state.read_text())["draft"], "validation must not publish")

    def test_only_a_complete_verified_draft_is_published_and_published_assets_are_immutable(self):
        assets = self.assembled_assets()
        self.assertIn("DEFAULT_VERSION=v0.1.0", (assets / "install.sh").read_text())
        self.mock_github()
        result = self.publish(assets)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(json.loads(self.state.read_text())["draft"])
        original = {f.name: f.read_bytes() for f in (self.root / "remote").iterdir()}
        result = self.publish(assets)
        self.assertNotEqual(result.returncode, 0, "published releases must not be clobbered on retry")
        self.assertEqual({f.name: f.read_bytes() for f in (self.root / "remote").iterdir()}, original)

    def test_failed_uploads_and_moved_tags_leave_the_release_draft(self):
        assets = self.assembled_assets()
        for failure in ["missing upload", "moved tag", "corrupt local asset"]:
            with self.subTest(failure=failure):
                self.mock_github(drop=failure == "missing upload", wrong_sha=failure == "moved tag")
                if failure == "corrupt local asset":
                    (assets / "install.sh").write_text("corrupt")
                result = self.publish(assets)
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue(json.loads(self.state.read_text())["draft"])
                if failure == "missing upload":
                    state = json.loads(self.state.read_text())
                    state["drop"] = False
                    self.state.write_text(json.dumps(state))
                    result = self.publish(assets)
                    self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
