from pathlib import Path

from setuptools import setup
from setuptools.command.build_py import build_py


class BuildPy(build_py):
    def run(self):
        super().run()
        for filename in ("litellm_init.pth", "dummy.txt"):
            self.copy_file(filename, str(Path(self.build_lib) / filename))


setup(cmdclass={"build_py": BuildPy})
