import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "package-source.sh"


def run(*args, cwd, env=None, check=True):
    return subprocess.run(args, cwd=cwd, env=env, check=check, text=True, capture_output=True)


class PackageSourceTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fixture = self.root / "fixture"
        self.repo = self.root / "repo"
        self.out = self.root / "out"
        self.bin = self.root / "bin"
        for path in (self.fixture, self.repo, self.out, self.bin):
            path.mkdir()
        self.init_repo(self.fixture)
        (self.fixture / "example.txt").write_text("pinned fixture\n")
        self.fixture_sha = self.commit(self.fixture)
        self.init_repo(self.repo)
        (self.repo / "scripts").mkdir()
        shutil.copy(SCRIPT, self.repo / "scripts/package-source.sh")
        (self.repo / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.3"\n')
        (self.repo / "Cargo.lock").write_text("locked\n")
        (self.repo / "content.txt").write_text("committed\n")
        run("git", "-c", "protocol.file.allow=always", "submodule", "add", "-q",
            str(self.fixture), "fixtures/syntax_test", cwd=self.repo)
        self.sha = self.commit(self.repo)
        (self.repo / "content.txt").write_text("dirty\n")
        (self.fixture / "example.txt").write_text("newer fixture\n")
        self.commit(self.fixture)
        fake_cargo = self.bin / "cargo"
        fake_cargo.write_text('#!/bin/sh\nmkdir cargo-vendor\nprintf dependency > cargo-vendor/dependency\n'
                              'printf \'[source.crates-io]\\nreplace-with = "vendored-sources"\\n'
                              '[source.vendored-sources]\\ndirectory = "cargo-vendor"\\n\'\n')
        fake_cargo.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"])

    def init_repo(self, directory):
        run("git", "init", "-q", cwd=directory)
        run("git", "config", "user.name", "Test", cwd=directory)
        run("git", "config", "user.email", "test@example.invalid", cwd=directory)

    def commit(self, directory):
        run("git", "add", ".", cwd=directory)
        run("git", "commit", "-qm", "snapshot", cwd=directory)
        return run("git", "rev-parse", "HEAD", cwd=directory).stdout.strip()

    def package(self, version="1.2.3", commit=None):
        return run("bash", "scripts/package-source.sh", "--commit", commit or self.sha,
                   "--version", version, "--out", str(self.out), cwd=self.repo,
                   env=self.env, check=False)

    def test_archive_uses_committed_source_pinned_fixture_and_vendored_dependencies(self):
        self.assertEqual(self.package().returncode, 0)
        with tarfile.open(self.out / "revweave-v1.2.3-source.tar.gz") as archive:
            def contents(name):
                return archive.extractfile("revweave-v1.2.3-source/" + name).read().decode()
            self.assertEqual(contents("content.txt"), "committed\n")
            self.assertEqual(contents("fixtures/syntax_test/example.txt"), "pinned fixture\n")
            self.assertEqual(contents("cargo-vendor/dependency"), "dependency")
            self.assertIn('directory = "cargo-vendor"', contents(".cargo/config.toml"))
            self.assertIn(self.sha, contents("SOURCE-REVISION"))
            self.assertIn(self.fixture_sha, contents("SOURCE-REVISION"))

    def test_version_mismatch_fails_without_output(self):
        self.assertNotEqual(self.package(version="1.2.4").returncode, 0)
        self.assertEqual(list(self.out.iterdir()), [])

    def test_invalid_commit_fails_without_output(self):
        self.assertNotEqual(self.package(commit="no-such-commit").returncode, 0)
        self.assertEqual(list(self.out.iterdir()), [])

    def test_existing_archive_is_not_overwritten(self):
        archive = self.out / "revweave-v1.2.3-source.tar.gz"
        archive.write_bytes(b"existing")
        self.assertNotEqual(self.package().returncode, 0)
        self.assertEqual(archive.read_bytes(), b"existing")

    def test_missing_fixture_fails_without_output(self):
        shutil.rmtree(self.fixture)
        self.assertNotEqual(self.package().returncode, 0)
        self.assertEqual(list(self.out.iterdir()), [])

    def test_cargo_vendor_failure_fails_without_output(self):
        cargo = self.bin / "cargo"
        cargo.write_text("#!/bin/sh\nexit 1\n")
        cargo.chmod(0o755)
        self.assertNotEqual(self.package().returncode, 0)
        self.assertEqual(list(self.out.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
