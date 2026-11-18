#!/usr/bin/env python3
"""Compare supported unified diffs with GNU patch and check SAT boundaries.

This is our integration suite, not a claim to pass the full GNU test suite.
Subprocess orchestration runs on the trusted host. Local SAT calls use run_sat.py.
"""
import argparse
import difflib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
WRAPPER = Path.home() / ".codex/skills/sat/scripts/run_sat.py"


def sat_command(sat, args):
    if WRAPPER.is_file():
        return [sys.executable, str(WRAPPER), "--", *args]
    return [str(sat), *args]


def unified(old, new, name="f"):
    return b"".join(difflib.diff_bytes(difflib.unified_diff,
        old.splitlines(keepends=True), new.splitlines(keepends=True),
        fromfile=f"a/{name}".encode(), tofile=f"b/{name}".encode()))


def invoke(sat, root, diff, flags=(), grants=None):
    args = ["patch", "-s", "-F0", "-p1", "-d", str(root), *flags, "--"]
    args += grants if grants is not None else ["--write", str(root)]
    env = dict(os.environ, SAT_BINARY=str(sat))
    return subprocess.run(sat_command(sat, args), input=diff, capture_output=True, env=env, timeout=30)


def snapshot(root):
    return {p.relative_to(root).as_posix(): p.read_bytes()
            for p in root.rglob("*") if p.is_file() and not p.is_symlink()}


def seed(root, files):
    for name, data in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sat", type=Path, required=True)
    parser.add_argument("--gnu-patch", default=shutil.which("patch"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    sat = args.sat.resolve()
    if not args.gnu_patch:
        parser.error("GNU patch is required")
    version = subprocess.check_output([args.gnu_patch, "--version"], text=True).splitlines()[0]
    assert "GNU patch" in version, version
    cases = []
    def case(name, files, diff, expected, flags=()):
        cases.append((name, files, diff, expected, flags))
    old, new = b"one\ntwo\nthree\n", b"one\nTWO\nthree\n"
    diff = unified(old, new)
    case("modify", {"f":old}, diff, {"f":new})
    case("reverse", {"f":new}, diff, {"f":old}, ["-R"])
    case("dry-run", {"f":old}, diff, {"f":old}, ["--dry-run"])
    case("offset", {"f":b"preface\n"+old}, diff, {"f":b"preface\n"+new})
    case("nested", {"src/f":old}, unified(old,new,"src/f"), {"src/f":new})
    case("multiple-files", {"f":old,"g":old}, diff+unified(old,new,"g"), {"f":new,"g":new})
    create = b"--- /dev/null\n+++ b/f\n@@ -0,0 +1 @@\n+hello\n"
    delete = b"--- a/f\n+++ /dev/null\n@@ -1 +0,0 @@\n-hello\n"
    case("create", {}, create, {"f":b"hello\n"})
    case("delete", {"f":b"hello\n"}, delete, {})
    case("reverse-create", {"f":b"hello\n"}, create, {}, ["-R"])
    case("reverse-delete", {}, delete, {"f":b"hello\n"}, ["-R"])
    case("dry-create", {}, create, {}, ["--dry-run"])
    case("empty-content", {"f":old}, unified(old,b""), {"f":b""})
    no_nl = b"--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n"
    case("no-final-newline", {"f":b"old"}, no_nl, {"f":b"new"})
    case("non-UTF8-content", {"f":b"\xff old\n"}, unified(b"\xff old\n",b"\xfe new\n"), {"f":b"\xfe new\n"})
    case("CRLF", {"f":b"old\r\n"}, unified(b"old\r\n",b"new\r\n"), {"f":b"new\r\n"}, ["--binary"])
    case("git-header", {"f":old}, b"diff --git a/f b/f\nindex 1234567..7654321 100644\n"+diff, {"f":new})
    case("email-preamble", {"f":old}, b"From example\nSubject: change\n\n"+diff, {"f":new})
    case("empty-input", {"f":old}, b"", {"f":old})
    records = []
    with tempfile.TemporaryDirectory(prefix="sat-patch-test-") as temp:
        base = Path(temp)
        for index, (name, files, content, expected, flags) in enumerate(cases):
            native, wasi = base/f"{index}-gnu", base/f"{index}-wasi"
            native.mkdir(); wasi.mkdir()
            seed(native,files); seed(wasi,files)
            command = [args.gnu_patch,"-s","-t","-F0","-p1","--no-backup-if-mismatch","-d",str(native),*flags]
            reference = subprocess.run(command,input=content,capture_output=True,timeout=30)
            result = invoke(sat,wasi,content,flags)
            assert reference.returncode == 0, (name,reference.stderr)
            assert result.returncode == 0, (name,result.stderr.decode(errors="replace"))
            assert snapshot(native) == snapshot(wasi) == expected, (name,snapshot(native),snapshot(wasi))
            records.append({"name":name,"status":"passed","kind":"GNU differential"})

        def check(name, files, content, *, flags=(), grant="read", setup=None, success=False, expected=None):
            root = base/name; root.mkdir(); seed(root,files)
            if setup: setup(root)
            before = snapshot(root)
            grants = ["--"+grant,str(root)] if grant else []
            result = invoke(sat,root,content,flags,grants)
            assert (result.returncode == 0) == success, (name,result.returncode,result.stderr.decode(errors="replace"))
            assert snapshot(root) == (before if expected is None else expected), name
            records.append({"name":name,"status":"passed","kind":"boundary/profile","exit":result.returncode})

        check("read-only-dry-run",{"f":old},diff,flags=["--dry-run"],success=True)
        check("read-only-write-denied",{"f":old},diff)
        check("no-grant-read-denied",{"f":old},diff,grant=None)
        check("conflict-no-write",{"f":b"wrong\n"},diff,grant="write")
        check("later-conflict-no-write",{"f":old,"g":b"wrong\n"},diff+unified(old,new,"g"),grant="write")
        check("traversal",{"f":old},diff.replace(b"a/f",b"a/../f"),grant="write")
        check("absolute-header",{"f":old},diff.replace(b"a/f",b"/tmp/f"),grant="write")
        check("windows-header",{"f":old},diff.replace(b"a/f",b"C:/f"),grant="write")
        check("strip-too-many",{"f":old},diff,flags=["-p2"],grant="write")
        check("malformed",{"f":old},b"not a patch\n",grant="write")
        check("duplicate-target",{"f":old},diff+diff,grant="write")
        check("unsupported-fuzz",{"f":old},diff,flags=["-F1"],grant="write")
        check("rename-rejected",{"f":old},b"diff --git a/f b/g\nsimilarity index 100%\nrename from f\nrename to g\n",grant="write")
        check("mode-rejected",{"f":old},b"diff --git a/f b/f\nold mode 100644\nnew mode 100755\n",grant="write")
        check("binary-rejected",{"f":old},b"diff --git a/f b/f\nBinary files a/f and b/f differ\n",grant="write")
        check("symlink-rejected",{"g":old},diff,grant="write",setup=lambda r:(r/"f").symlink_to("g"))
        check("oversized-patch",{"f":old},b"x"*(16*1024*1024+1),grant="write")
        # Exact file grants should suffice for ordinary modification.
        root=base/"exact-grant"; root.mkdir(); seed(root,{"f":old,"g":old})
        result=invoke(sat,root,diff,grants=["--write",str(root/"f")])
        assert result.returncode==0, result.stderr
        assert snapshot(root)=={"f":new,"g":old}
        records.append({"name":"exact-file-write","status":"passed","kind":"boundary/profile"})
        # A denied second file must not leave the first file modified.
        (root/"f").write_bytes(old)
        result=invoke(sat,root,diff+unified(old,new,"g"),grants=["--write",str(root/"f"),"--read",str(root/"g")])
        assert result.returncode!=0
        assert snapshot(root)=={"f":old,"g":old}
        records.append({"name":"later-write-denial-no-write","status":"passed","kind":"boundary/profile"})
    report={"gnu_patch":version,"engine":"diffy 0.5.2","passed":len(records),"tests":records}
    if args.output:
        args.output.parent.mkdir(parents=True,exist_ok=True)
        args.output.write_text(json.dumps(report,indent=2)+"\n")
    print(f"{len(records)} checks passed ({len(cases)} GNU differential cases).")


if __name__ == "__main__":
    main()
