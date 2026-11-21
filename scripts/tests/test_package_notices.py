import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[2]
NOTICES = ROOT / "BUNDLED-LICENSE-NOTICES.txt"


class PackageNoticesTest(unittest.TestCase):
    def test_notice_inventory_is_current(self):
        subprocess.run([sys.executable, "scripts/generate-bundled-license-notices.py", "--check"],
                       cwd=ROOT, check=True)
        contents = NOTICES.read_text(encoding="utf-8")
        for source in (
            "vendor/difftastic/LICENSE",
            "vendor/tree-sitter-razor-modern/NOTICE",
            "third-party-licenses/tree-sitter-wat/Apache-2.0.txt",
            "third-party-licenses/tree-sitter-wat/LLVM-exception.txt",
            "crates/gitcomet-fonts/assets/fira_code/LICENSE-OFL.txt",
            "crates/gitcomet-fonts/assets/ibm_plex_sans/LICENSE-OFL.txt",
            "crates/gitcomet-fonts/assets/lilex/LICENSE-OFL.txt",
            "crates/gitcomet-fonts/assets/noto_emoji/OFL.txt",
        ):
            self.assertIn((ROOT / source).read_text(encoding="utf-8").rstrip(), contents)

    def test_linux_tarball_contains_notices(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            payload = work / "payload"
            binary = work / "revweave"
            binary.write_text("fixture\n")
            binary.chmod(0o755)
            output = work / "output"
            output.mkdir()
            env = os.environ.copy()
            shim = work / "bin"
            shim.mkdir()
            validator = shim / "desktop-file-validate"
            validator.write_text("#!/bin/sh\nexit 0\n")
            validator.chmod(0o755)
            env["PATH"] = str(shim) + os.pathsep + env["PATH"]
            if sys.platform == "darwin":
                # The production script targets GNU install on Linux.
                install = shim / "install"
                install.write_text('#!/bin/bash\nif [[ "$1" == -Dm* ]]; then '
                                   'mode="${1#-D}"; shift; mkdir -p "$(dirname "$2")"; '
                                   'exec /usr/bin/install "-$mode" "$@"; fi\n'
                                   'exec /usr/bin/install "$@"\n')
                install.chmod(0o755)
            subprocess.run(["bash", "scripts/package-linux.sh", "stage", "--binary", str(binary),
                            "--source", str(ROOT), "--out", str(payload)],
                           cwd=ROOT, env=env, check=True)
            self.assertEqual((payload / "usr/share/licenses/revweave" / NOTICES.name).read_bytes(),
                             NOTICES.read_bytes())
            subprocess.run(["bash", "scripts/package-linux.sh", "tarball", "--payload", str(payload),
                            "--version", "1.2.3", "--arch", "x86_64", "--out", str(output)],
                           cwd=ROOT, env=env, check=True)
            import tarfile
            with tarfile.open(output / "revweave-v1.2.3-linux-x86_64.tar.gz") as archive:
                notice = archive.extractfile("revweave-v1.2.3-linux-x86_64/" + NOTICES.name)
                self.assertEqual(notice.read(), NOTICES.read_bytes())

    def test_windows_installer_references_notice(self):
        root = ET.parse(ROOT / "crates/gitcomet/wix/main.wxs")
        namespace = {"w": "http://schemas.microsoft.com/wix/2006/wi"}
        files = root.findall(".//w:Component[@Id='BundledLicenseNotices']/w:File", namespace)
        self.assertEqual([entry.get("Source") for entry in files], [NOTICES.name])
        self.assertIsNotNone(root.find(".//w:Feature/w:ComponentRef[@Id='BundledLicenseNotices']", namespace))


if __name__ == "__main__":
    unittest.main()
