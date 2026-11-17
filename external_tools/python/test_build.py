"""Verify that source changes and artifact tampering invalidate the build cache."""
import contextlib
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import build


class BuildCacheTests(unittest.TestCase):
    def test_cache_requires_matching_sources_builder_and_digest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / "target"
            executables = target / "builder/bin"
            executables.mkdir(parents=True)
            (executables / "python").touch()
            (target / "builder/Scripts").mkdir()
            (target / "builder/Scripts/python.exe").touch()
            (root / "wit").mkdir()
            (root / "app.py").write_text("# wrapper one\n")
            (root / "sat_http.py").write_text("# transport one\n")
            (root / "build.py").write_text("# builder\n")
            (root / "wit/world.wit").write_text("world test {}\n")
            builds = []

            def command(args, **kwargs):
                if "componentize" in args:
                    builds.append(args)
                    Path(args[-1]).write_bytes(b"component " + str(len(builds)).encode())
                return subprocess.CompletedProcess(args, 0, stdout=build.BUILDER)

            with (patch.object(build, "ROOT", root), patch.object(build, "TARGET", target),
                  patch.object(build.subprocess, "run", side_effect=command),
                  patch.object(build.subprocess, "check_output", return_value="Python 3.12.0"),
                  contextlib.redirect_stdout(io.StringIO())):
                artifact = build.build()
                build.build()
                self.assertEqual(len(builds), 1)
                (root / "app.py").write_text("# wrapper two\n")
                build.build()
                self.assertEqual(len(builds), 2)
                (root / "wit/world.wit").write_text("world changed {}\n")
                build.build()
                self.assertEqual(len(builds), 3)
                (root / "sat_http.py").write_text("# transport two\n")
                build.build()
                self.assertEqual(len(builds), 4)
                (root / "wit/deps").mkdir()
                (root / "wit/deps/http.wit").write_text("package wasi:http;\n")
                build.build()
                self.assertEqual(len(builds), 5)
                artifact.write_bytes(b"tampered")
                build.build()
                self.assertEqual(len(builds), 6)
                with patch.object(build, "BUILDER", "different-version"):
                    build.build()
                self.assertEqual(len(builds), 7)


if __name__ == "__main__":
    unittest.main()
