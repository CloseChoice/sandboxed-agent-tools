"""Release selection and version checks must fail closed before publishing."""
import contextlib
import io
from unittest import mock
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("release", Path(__file__).with_name("release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.write("sat/Cargo.toml", '[package]\nname="sat"\nversion="0.1.0"\n')
        self.write("sat/Cargo.lock", '[[package]]\nname="sat"\nversion="0.1.0"\n')
        self.write("external_tools/release-tools.toml", 'tools=["python"]\n')
        self.write("external_tools/python/tool.toml", 'name="python"\nversion="0.1.0"\n')
        self.write("external_tools/python/defaults.toml", "")
        self.write("external_tools/python/package.sh", "")

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def test_selected_python_tool_without_cargo(self):
        self.assertEqual(release.validate(self.root, "v0.1.0"), ("0.1.0", ["python"]))

    def test_tag_must_match_host_version(self):
        with self.assertRaisesRegex(ValueError, "release tag"):
            release.validate(self.root, "v0.2.0")

    def test_lockfile_must_match(self):
        self.write("sat/Cargo.lock", '[[package]]\nname="sat"\nversion="0.2.0"\n')
        with self.assertRaisesRegex(ValueError, "Cargo.lock"):
            release.validate(self.root)

    def test_invalid_and_duplicate_selections(self):
        for selection in ['["../python"]', '["python","python"]', '[]']:
            with self.subTest(selection=selection):
                self.write("external_tools/release-tools.toml", f"tools={selection}\n")
                with self.assertRaises(ValueError):
                    release.validate(self.root)

    def test_missing_selected_tool_is_rejected(self):
        self.write("external_tools/release-tools.toml", 'tools=["missing"]\n')
        with self.assertRaises(OSError):
            release.validate(self.root)

    def test_rust_metadata_must_match(self):
        self.write("external_tools/python/Cargo.toml", '[package]\nname="python"\nversion="0.2.0"\n')
        self.write("external_tools/python/Cargo.lock", '[[package]]\nname="python"\nversion="0.2.0"\n')
        with self.assertRaisesRegex(ValueError, "tool.toml"):
            release.validate(self.root)


    def test_complete_archive_set_rejects_extra_and_missing_tools(self):
        archive_path = Path(__file__).resolve().parents[1] / "external_tools/check_archives.py"
        archive_spec = importlib.util.spec_from_file_location("archives", archive_path)
        archives = importlib.util.module_from_spec(archive_spec)
        archive_spec.loader.exec_module(archives)
        for names in [["python.tar", "extra.tar"], ["extra.tar"]]:
            with self.subTest(names=names), mock.patch.object(archives, "ROOT", self.root / "external_tools"):
                with mock.patch("sys.argv", ["check_archives.py", "--complete", *names]):
                    stderr = io.StringIO()
                    with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit) as failure:
                        archives.main()
                    self.assertEqual(failure.exception.code, 2)
                    self.assertIn("incomplete release", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
