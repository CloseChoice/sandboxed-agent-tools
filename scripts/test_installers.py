"""Installer integration checks with local release fixtures; no real downloads or installs."""
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PWSH = os.environ.get("SAT_TEST_PWSH") or shutil.which("pwsh")
VERSION = "0.1.0-rc.1"
REPO = "https://github.com/CloseChoice/sandboxed-agent-tools"
PLATFORMS = {"linux-x64": "x86_64-unknown-linux-gnu",
             "macos-x64": "x86_64-apple-darwin",
             "macos-arm64": "aarch64-apple-darwin"}


@unittest.skipIf(os.name == "nt", "Fixtures use a Unix executable stub; run on the Linux CI job")
class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="sat install test ")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.fixtures = self.root / "release"
        self.fixtures.mkdir()
        self.cli_log = self.root / "cli.log"
        self.download_log = self.root / "downloads.log"
        self.env = dict(os.environ, SAT_FIXTURE_ROOT=str(self.fixtures),
                        SAT_INSTALL_TEST_LOG=str(self.cli_log), SAT_DOWNLOAD_LOG=str(self.download_log),
                        XDG_CACHE_HOME=str(self.root / "cache"),
                        XDG_CONFIG_HOME=str(self.root / "config"),
                        XDG_DATA_HOME=str(self.root / "data"), POWERSHELL_TELEMETRY_OPTOUT="1")
        binary = b"""#!/usr/bin/env bash
set -eu
printf '%s|%s\n' "$PWD" "$*" >> "$SAT_INSTALL_TEST_LOG"
for i in 1 2 3 4 5; do
    IFS= read -r line
    [[ "$line" == "[]" ]] || exit 9
done
IFS= read -r line
[[ "$line" == "unset" ]] || exit 9
"""
        for target in PLATFORMS.values():
            with tarfile.open(self.fixtures / f"sat-{VERSION}-{target}.tar.gz", "w:gz") as archive:
                member = tarfile.TarInfo("sat")
                member.mode = 0o755
                member.size = len(binary)
                archive.addfile(member, io.BytesIO(binary))
        with zipfile.ZipFile(self.fixtures / f"sat-{VERSION}-x86_64-pc-windows-msvc.zip", "w") as archive:
            archive.writestr("sat.exe", binary)
        for name in ["python", "jaq", "code", "markitdown", "ripgrep"]:
            (self.fixtures / f"{name}.tar").write_bytes(b"tool fixture: " + name.encode())
        sums = "".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n"
                       for p in sorted(self.fixtures.iterdir()))
        (self.fixtures / "SHA256SUMS").write_text(sums)
        fake_bin = self.root / "fake bin"
        fake_bin.mkdir()
        curl = fake_bin / "curl"
        curl.write_text("#!" + sys.executable + "\n" + """
import os
from pathlib import Path
import shutil
import sys
args = sys.argv[1:]
url = next(a for a in args if a.startswith("https://"))
with open(os.environ["SAT_DOWNLOAD_LOG"], "a") as log:
    print(url, file=log)
if url.endswith("/releases/latest"):
    print("https://github.com/CloseChoice/sandboxed-agent-tools/releases/tag/v0.1.0-rc.1", end="")
else:
    shutil.copyfile(Path(os.environ["SAT_FIXTURE_ROOT"]) / url.rsplit("/", 1)[1],
                    args[args.index("-o") + 1])
""")
        curl.chmod(0o755)
        self.env["PATH"] = str(fake_bin) + os.pathsep + os.environ["PATH"]
        self.harness = self.root / "harness.ps1"
        self.harness.write_text("""
function Invoke-RestMethod {
    param($Uri)
    Add-Content -LiteralPath $env:SAT_DOWNLOAD_LOG -Value $Uri
    return @{ tag_name = "v0.1.0-rc.1" }
}
function Invoke-WebRequest {
    param($Uri, $OutFile, [switch]$UseBasicParsing)
    Add-Content -LiteralPath $env:SAT_DOWNLOAD_LOG -Value $Uri
    $name = ([Uri]$Uri).Segments[-1]
    Copy-Item -LiteralPath (Join-Path $env:SAT_FIXTURE_ROOT $name) -Destination $OutFile
}
function Expand-Archive {
    param($LiteralPath, $DestinationPath)
    Microsoft.PowerShell.Archive\\Expand-Archive -LiteralPath $LiteralPath -DestinationPath $DestinationPath
    # The fake Windows binary is a Unix stub for this local fixture test only.
    & chmod +x (Join-Path $DestinationPath "sat.exe")
}
$settings = Get-Content -Raw $env:SAT_PARAMETERS_FILE | ConvertFrom-Json -AsHashtable
& $env:SAT_INSTALL_SCRIPT @settings
""")

    def backends(self):
        return ["bash"] + (["powershell"] if PWSH else [])

    def run_installer(self, backend, tools="", version=VERSION, platform=None):
        destination = self.root / "installed bin" / backend
        env = dict(self.env)
        if backend == "bash":
            platform = platform or "linux-x64"
            args = ["--bin-dir", str(destination), "--version", version, "--platform", platform]
            if tools:
                args.append(tools)
            # Exercise the documented pipe-to-bash form with installer arguments.
            command = ["bash", "-s", "--", *args]
            result = subprocess.run(command, input=(ROOT / "install.sh").read_text(),
                                    env=env, cwd=self.root, capture_output=True, text=True)
            binary = destination / "sat"
        else:
            settings = self.root / "settings.json"
            settings.write_text(json.dumps({"Tools": tools, "Version": version,
                                             "Platform": platform or "windows-x64", "BinDir": str(destination)}))
            env.update(SAT_PARAMETERS_FILE=str(settings), SAT_INSTALL_SCRIPT=str(ROOT / "install.ps1"))
            result = subprocess.run([PWSH, "-NoProfile", "-NonInteractive", "-File", str(self.harness)],
                                    env=env, cwd=self.root, capture_output=True, text=True)
            binary = destination / "sat.exe"
        return result, binary

    def assert_ok(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_host_only_and_latest_resolution(self):
        for backend in self.backends():
            with self.subTest(backend=backend):
                result, binary = self.run_installer(backend, version="latest")
                self.assert_ok(result)
                self.assertTrue(binary.exists())
        self.assertFalse(self.cli_log.exists())
        self.assertIn("/releases/download/v" + VERSION, self.download_log.read_text())

    def test_selected_tools_get_empty_permissions_outside_callers_project(self):
        for backend in self.backends():
            with self.subTest(backend=backend):
                self.cli_log.unlink(missing_ok=True)
                result, _ = self.run_installer(backend, "python,jaq", version="v" + VERSION)
                self.assert_ok(result)
                calls = self.cli_log.read_text().splitlines()
                self.assertEqual(len(calls), 2)
                self.assertIn("python.tar", calls[0])
                self.assertIn("jaq.tar", calls[1])
                self.assertTrue(all(not line.startswith(str(self.root) + "|") for line in calls))
                self.assertFalse((self.root / ".sat").exists())

    def test_default_tools(self):
        for backend in self.backends():
            with self.subTest(backend=backend):
                self.cli_log.unlink(missing_ok=True)
                result, _ = self.run_installer(backend, "default-tools")
                self.assert_ok(result)
                calls = self.cli_log.read_text().splitlines()
                self.assertEqual(len(calls), 5)
                for name in ["python", "jaq", "code", "markitdown", "ripgrep"]:
                    self.assertTrue(any(name + ".tar" in line for line in calls))

    def test_explicit_unix_platforms(self):
        for platform, target in PLATFORMS.items():
            with self.subTest(platform=platform):
                result, _ = self.run_installer("bash", platform=platform)
                self.assert_ok(result)
                self.assertIn(f"/sat-{VERSION}-{target}.tar.gz", self.download_log.read_text())

    def test_unknown_tools_and_invalid_versions_fail_before_downloading(self):
        for backend in self.backends():
            for tools, version in [("unknown", VERSION), ("python", "../../invalid")]:
                with self.subTest(backend=backend, tools=tools, version=version):
                    result, binary = self.run_installer(backend, tools, version=version)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(binary.exists())
                    self.assertFalse(self.download_log.exists())

    def test_checksum_failure_keeps_existing_binary(self):
        for backend in self.backends():
            with self.subTest(backend=backend):
                extension = "x86_64-pc-windows-msvc.zip" if backend == "powershell" else "x86_64-unknown-linux-gnu.tar.gz"
                asset = self.fixtures / f"sat-{VERSION}-{extension}"
                asset.write_bytes(asset.read_bytes() + b"corruption")
                destination = self.root / "installed bin" / backend
                destination.mkdir(parents=True)
                binary = destination / ("sat.exe" if backend == "powershell" else "sat")
                binary.write_bytes(b"existing host")
                result, _ = self.run_installer(backend)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("checksum", (result.stdout + result.stderr).lower())
                self.assertEqual(binary.read_bytes(), b"existing host")
                self.assertFalse(self.cli_log.exists())


class PowerShellSyntaxTests(unittest.TestCase):
    @unittest.skipUnless(PWSH, "PowerShell is not available")
    def test_parse(self):
        temporary = tempfile.TemporaryDirectory(prefix="sat-pwsh-syntax-")
        self.addCleanup(temporary.cleanup)
        env = dict(os.environ, SAT_INSTALL_SCRIPT=str(ROOT / "install.ps1"),
                   XDG_CACHE_HOME=temporary.name, XDG_CONFIG_HOME=temporary.name,
                   XDG_DATA_HOME=temporary.name, POWERSHELL_TELEMETRY_OPTOUT="1")
        command = """$errors=$null; $tokens=$null
[System.Management.Automation.Language.Parser]::ParseFile($env:SAT_INSTALL_SCRIPT,[ref]$tokens,[ref]$errors) | Out-Null
if ($errors.Count) { $errors | Out-String | Write-Error; exit 1 }
"""
        result = subprocess.run([PWSH, "-NoProfile", "-NonInteractive", "-Command", command],
                                env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
