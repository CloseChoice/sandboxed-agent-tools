"""Build a pinned upstream MarkItDown PDF CLI profile, entirely inside WASI."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import venv

ROOT = Path(__file__).resolve().parent
TARGET = ROOT / 'target'
VERSIONS = {'componentize-py': '0.25.0', 'pdfminer.six': '20260107',
            'pdfplumber': '0.11.10', 'charset-normalizer': '3.5.2'}


def build():
    TARGET.mkdir(exist_ok=True)
    builder = TARGET / 'builder'
    scripts = builder / ('Scripts' if os.name == 'nt' else 'bin')
    python = scripts / ('python.exe' if os.name == 'nt' else 'python')
    if not python.exists():
        venv.EnvBuilder(with_pip=True).create(builder)
    probe = subprocess.run([str(python), '-c',
        'import importlib.metadata,json; print(json.dumps({n:importlib.metadata.version(n) for n in ' + repr(list(VERSIONS)) + '}))'],
        capture_output=True, text=True)
    if probe.returncode or json.loads(probe.stdout) != VERSIONS:
        # The supported PDF path uses these Python sources. Do not install the
        # native renderer/crypto/ML extras which cannot execute in this component.
        subprocess.run([str(python), '-m', 'pip', 'install', '--no-deps',
                        *[f'{name}=={version}' for name, version in VERSIONS.items()]], check=True)
    source_names = ['app.py', 'pdf_profile.py', 'wit/world.wit', 'build.py']
    source_names += [str(p.relative_to(ROOT)) for p in sorted((ROOT / 'upstream').rglob('*')) if p.is_file()]
    spec = {'versions': VERSIONS, 'host_python': subprocess.check_output([str(python), '--version'], text=True),
            'sources': {n: hashlib.sha256((ROOT / n).read_bytes()).hexdigest() for n in source_names}}
    fingerprint = hashlib.sha256(json.dumps(spec, sort_keys=True).encode()).hexdigest()
    output = TARGET / 'markitdown.wasm'
    record = TARGET / 'build.json'
    try:
        cached = json.loads(record.read_text())
    except (OSError, ValueError):
        cached = {}
    if output.exists() and cached.get('spec') == fingerprint and cached.get('sha256') == hashlib.sha256(output.read_bytes()).hexdigest():
        print('Reusing verified MarkItDown component.', flush=True)
        return output
    profile = TARGET / 'profile'
    if profile.exists():
        shutil.rmtree(profile)
    shutil.copytree(ROOT / 'upstream/markitdown', profile / 'markitdown')
    (profile / 'markitdown/__init__.py').write_text('from ._markitdown import MarkItDown, StreamInfo, DocumentConverterResult\n')
    (profile / 'markitdown/converters/__init__.py').write_text('')
    shutil.copy2(ROOT / 'pdf_profile.py', profile / 'markitdown/_markitdown.py')
    for name in ['pdfminer', 'pdfplumber', 'charset_normalizer']:
        location = subprocess.check_output([str(python), '-c',
            f'import importlib.util; print(next(iter(importlib.util.find_spec({name!r}).submodule_search_locations)))'], text=True).strip()
        shutil.copytree(location, profile / name, ignore=shutil.ignore_patterns('*.so', '*.pyd', '__pycache__', '*.pyc'))
    crypto = profile / 'pdfminer/pdfdocument.py'
    content = crypto.read_text()
    imports = 'from cryptography.hazmat.backends import default_backend\nfrom cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes'
    assert imports in content, 'Pinned pdfminer crypto imports changed'
    content = content.replace(imports, 'default_backend = Cipher = algorithms = modes = None')
    needle = '    def _initialize_password(self, password: str = "") -> None:\n'
    assert needle in content, 'Pinned encryption entry point changed'
    content = content.replace(needle, needle + '        raise PDFEncryptionError("Encrypted PDFs are unavailable in this WASI profile")\n', 1)
    crypto.write_text(content)
    repair = profile / 'pdfplumber/repair.py'
    content = repair.read_text().replace('import subprocess\n', '')
    needle = ') -> BytesIO:\n'
    assert needle in content, 'Pinned Ghostscript repair entry point changed'
    content = content.replace(needle, needle + '    raise NotImplementedError("Ghostscript repair is unavailable in WASI")\n', 1)
    repair.write_text(content)
    componentize = scripts / ('componentize-py.exe' if os.name == 'nt' else 'componentize-py')
    pending = TARGET / 'markitdown.pending.wasm'
    try:
        subprocess.run([str(componentize), '-d', 'wit', '-w', 'markitdown-cli',
            'componentize', '-p', str(profile), '-p', str(ROOT), 'app', '-o', str(pending)],
            cwd=ROOT, env=dict(os.environ, VIRTUAL_ENV=str(builder)), check=True)
        pending.replace(output)
    finally:
        pending.unlink(missing_ok=True)
    record.write_text(json.dumps({'spec': fingerprint, 'sha256': hashlib.sha256(output.read_bytes()).hexdigest()}))
    return output


if __name__ == '__main__':
    build()
