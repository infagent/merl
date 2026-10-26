"""Exercise the shipped installer against synthetic local release downloads."""

import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
    ("Linux", "aarch64"): "aarch64-unknown-linux-gnu",
    ("Darwin", "x86_64"): "x86_64-apple-darwin",
    ("Darwin", "arm64"): "aarch64-apple-darwin",
}


class Installer(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="merl installer ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.assets = self.root / "assets"
        self.assets.mkdir()
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.bin_dir = self.root / "user directory" / "bin"
        self.env = dict(os.environ, PATH=f"{self.tools}:/usr/bin:/bin",
                        FIXTURES=str(self.assets), REQUESTS=str(self.root / "requests"),
                        TEST_OS="Linux", TEST_ARCH="x86_64", TMPDIR=str(self.root))
        self.tool("uname", '#!/bin/sh\ncase "$1" in -s) echo "$TEST_OS";; -m) echo "$TEST_ARCH";; esac\n')
        self.tool("curl", '''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
a=sys.argv[1:]; url=a[-1]; log=pathlib.Path(os.environ["REQUESTS"])
with log.open("a") as f: f.write(json.dumps(url)+"\\n")
if url.endswith("/releases/latest"):
    print("https://github.com/infagent/merl/releases/tag/v0.1.0", end="")
else:
    try:
        tag, name=url.split("/releases/download/",1)[1].split("/",1)
        output=a[a.index("--output")+1]
        if os.environ.get("PARTIAL_DOWNLOAD") and name.endswith(".tar.gz"):
            pathlib.Path(output).write_bytes(b"partial download"); sys.exit(18)
        shutil.copyfile(pathlib.Path(os.environ["FIXTURES"])/tag/name, output)
    except (OSError, ValueError): sys.exit(22)
''')
        for target in TARGETS.values():
            self.release(target)

    def tool(self, name, text):
        path = self.tools / name
        path.write_text(text)
        path.chmod(0o755)

    def release(self, target, version="0.1.0", binary_version=None):
        directory = self.assets / f"v{version}"
        directory.mkdir(exist_ok=True)
        name = f"merl-v{version}-{target}.tar.gz"
        binary = f'#!/bin/sh\necho "merl {binary_version or version}"\n'.encode()
        with tarfile.open(directory / name, "w:gz") as archive:
            for member, content in {
                "merl/merl": binary, "merl/VERSION": version.encode(),
                "merl/REVISION": b"a" * 40, "merl/LICENSE": b"synthetic license",
                "merl/skills/merl-mail/SKILL.md": b"synthetic matching skill",
            }.items():
                info = tarfile.TarInfo(member)
                info.mode = 0o755 if member == "merl/merl" else 0o644
                info.size = len(content)
                archive.addfile(info, io.BytesIO(content))
        sums = directory / "SHA256SUMS"
        entries = [p for p in directory.glob("*.tar.gz")]
        sums.write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in sorted(entries)))
        return directory / name

    def install(self, *args):
        return subprocess.run(["sh", str(ROOT / "install.sh"), "--bin-dir", str(self.bin_dir), *args],
                              env=self.env, capture_output=True, text=True)

    def requests(self):
        path = self.root / "requests"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def prior_install(self):
        self.bin_dir.mkdir(parents=True)
        old = self.bin_dir / "merl"
        old.write_bytes(b"previous working installation")
        return old

    def test_each_supported_platform_installs_the_exact_tag_with_spaces_in_paths(self):
        for (system, arch), target in TARGETS.items():
            with self.subTest(system=system, arch=arch):
                self.env.update(TEST_OS=system, TEST_ARCH=arch)
                result = self.install("--version", "v0.1.0")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f"merl-v0.1.0-{target}.tar.gz", self.requests()[-1])
                self.assertEqual(subprocess.check_output([self.bin_dir / "merl", "--version"], text=True).strip(), "merl 0.1.0")
                self.assertIn(str(self.bin_dir), result.stdout)
                self.assertFalse(list(self.bin_dir.glob(".merl-install.*")))

    def test_latest_is_resolved_once_then_all_downloads_use_that_tag(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        urls = self.requests()
        self.assertEqual(sum(url.endswith("/releases/latest") for url in urls), 1)
        self.assertEqual(len(urls), 3)
        self.assertTrue(all("/v0.1.0/" in url for url in urls[1:]))

    def test_default_installation_uses_only_the_selected_user_home(self):
        home = self.root / "clean home"
        home.mkdir()
        env = dict(self.env, HOME=str(home))
        result = subprocess.run(["sh", str(ROOT / "install.sh"), "--version", "v0.1.0"],
                                env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((home / ".local/bin/merl").is_file())
        self.assertEqual(sorted(str(p.relative_to(home)) for p in home.rglob("*") if p.is_file()),
                         [".local/bin/merl"], "no profile, service, skill or agent configuration is installed")

    def test_failures_preserve_an_existing_binary_and_clean_owned_temporary_files(self):
        old = self.prior_install()
        archive = self.assets / "v0.1.0" / "merl-v0.1.0-x86_64-unknown-linux-gnu.tar.gz"
        for failure in ["checksum", "missing", "partial download", "wrong version", "rename"]:
            with self.subTest(failure=failure):
                self.release("x86_64-unknown-linux-gnu")
                self.env.pop("PARTIAL_DOWNLOAD", None)
                if failure == "checksum":
                    archive.write_bytes(b"corrupt download")
                elif failure == "missing":
                    archive.unlink()
                elif failure == "wrong version":
                    self.release("x86_64-unknown-linux-gnu", binary_version="9.9.9")
                elif failure == "partial download":
                    self.env["PARTIAL_DOWNLOAD"] = "1"
                else:
                    self.tool("mv", "#!/bin/sh\nexit 1\n")
                result = self.install("--version", "v0.1.0")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("merl-install:", result.stderr)
                self.assertEqual(old.read_bytes(), b"previous working installation")
                self.assertFalse(list(self.root.glob("merl-install.*")))
                self.assertFalse(list(self.bin_dir.glob(".merl-install.*")))

    def test_directories_and_symlinks_are_rejected_without_touching_their_targets(self):
        self.bin_dir.mkdir(parents=True)
        destination = self.bin_dir / "merl"
        destination.mkdir()
        result = self.install("--version", "v0.1.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(destination.is_dir())
        destination.rmdir()
        for target in [self.root / "missing", self.root / "existing"]:
            if target.name == "existing":
                target.write_text("keep")
            destination.symlink_to(target)
            result = self.install("--version", "v0.1.0")
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue(destination.is_symlink())
            destination.unlink()
        self.assertEqual((self.root / "existing").read_text(), "keep")
        self.assertEqual(self.requests(), [])

    def test_unsupported_platforms_and_invalid_versions_fail_before_downloading(self):
        self.env["TEST_OS"] = "Windows_NT"
        result = self.install("--version", "v0.1.0")
        self.assertNotEqual(result.returncode, 0)
        self.env["TEST_OS"] = "Linux"
        for version in ["main", "v1/../../escape", "", "v0.1.0;echo bad", "v0.1.0\nmain"]:
            result = self.install("--version", version)
            self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.requests(), [])
        self.assertFalse(self.bin_dir.exists())

    @unittest.skipIf(os.geteuid() == 0, "root bypasses directory write permissions")
    def test_unwritable_directory_preserves_the_previous_installation(self):
        old = self.prior_install()
        self.bin_dir.chmod(0o500)
        try:
            result = self.install("--version", "v0.1.0")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(old.read_bytes(), b"previous working installation")
        finally:
            self.bin_dir.chmod(0o700)


if __name__ == "__main__":
    unittest.main()
