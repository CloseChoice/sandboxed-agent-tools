#!/usr/bin/env python3
"""Run the unchanged GNU patch 2.8 no-mode-change-git-diff regression.

Only this selected upstream script is claimed; the full GNU suite is out of scope.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

HERE=Path(__file__).resolve().parent

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sat",type=Path,required=True)
    args=parser.parse_args()
    upstream=HERE/"upstream"
    manifest=json.loads((upstream/"manifest.json").read_text())
    for name,digest in manifest["files"].items():
        assert hashlib.sha256((upstream/name).read_bytes()).hexdigest()==digest,name
    with tempfile.TemporaryDirectory(prefix="sat-patch-gnu-") as tmp:
        root=Path(tmp)
        (root/"tests").mkdir()
        launcher=root/"patch"
        launcher.write_bytes((HERE/"upstream_launcher.py").read_bytes())
        launcher.chmod(0o755)
        env=dict(os.environ,srcdir=str(upstream),abs_top_builddir=str(root),
                 PATCH=str(launcher),SAT_BINARY=str(args.sat.resolve()))
        subprocess.run(["sh",str(upstream/"no-mode-change-git-diff")],env=env,check=True)

if __name__=="__main__":
    main()
