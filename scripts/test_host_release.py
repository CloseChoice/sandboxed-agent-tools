"""Check host archive integrity, executable permissions, and release completeness."""
import contextlib
import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("host_release", Path(__file__).with_name("host_release.py"))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class HostReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.patch = mock.patch.object(host, "version", return_value="0.1.0-rc.1")
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def pack(self, target):
        header = b"MZ" if "windows" in target else (
            b"\xcf\xfa\xed\xfe" if "apple" in target else b"\x7fELF")
        binary = self.root / host.binary_name(target)
        binary.write_bytes(header + b"fixture")
        with contextlib.redirect_stdout(io.StringIO()):
            return host.pack(target, binary, self.root / "output"), binary.read_bytes()

    def test_complete_release_and_round_trip_all_platforms(self):
        for entry in host.TARGETS:
            target = entry["target"]
            archive, data = self.pack(target)
            self.assertEqual(host.unpack_checked(archive, target), data)
            original = archive.read_bytes()
            self.pack(target)
            self.assertEqual(archive.read_bytes(), original)
        with contextlib.redirect_stdout(io.StringIO()):
            host.verify(self.root / "output")

    def test_missing_or_extra_host_asset_is_rejected(self):
        target = host.TARGETS[0]["target"]
        archive, _ = self.pack(target)
        with self.assertRaisesRegex(ValueError, "incomplete host release"):
            host.verify(archive.parent)
        for entry in host.TARGETS[1:]:
            self.pack(entry["target"])
        (archive.parent / "sat-unexpected.zip").write_bytes(b"unexpected")
        with self.assertRaisesRegex(ValueError, "unexpected"):
            host.verify(archive.parent)

    def test_unsafe_members_and_nonexecutable_permissions_are_rejected(self):
        target = host.TARGETS[0]["target"]
        archive, data = self.pack(target)
        for name, mode in [("../sat", 0o755), ("sat", 0o644)]:
            with self.subTest(name=name, mode=mode):
                with tarfile.open(archive, "w:gz") as package:
                    entry = tarfile.TarInfo(name)
                    entry.mode = mode
                    entry.size = len(data)
                    package.addfile(entry, io.BytesIO(data))
                with self.assertRaisesRegex(ValueError, "only its executable"):
                    host.unpack_checked(archive, target)

    def test_wrong_executable_format_is_rejected(self):
        binary = self.root / "not-an-executable"
        binary.write_bytes(b"text")
        with self.assertRaisesRegex(ValueError, "executable format"):
            host.pack(host.TARGETS[0]["target"], binary, self.root / "output")


if __name__ == "__main__":
    unittest.main()
