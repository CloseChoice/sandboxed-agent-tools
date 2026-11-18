#!/usr/bin/env python3
"""Grant the GNU test's temporary directory; keep upstream assertions unchanged."""
import os
from pathlib import Path
import subprocess
import sys

wrapper = Path.home()/".codex/skills/sat/scripts/run_sat.py"
prefix = [sys.executable,str(wrapper),"--"] if wrapper.is_file() else [os.environ["SAT_BINARY"]]
root = str(Path.cwd())
sys.exit(subprocess.run([*prefix,"patch","-d",root,*sys.argv[1:],"--","--write",root]).returncode)
