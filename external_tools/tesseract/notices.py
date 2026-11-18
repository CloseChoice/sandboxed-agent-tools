"""Keep dependency notices inside the component's distribution payload."""
from pathlib import Path
import hashlib,json,subprocess

def attach(root:Path,component:Path):
    sections=['# Third-party notices','',
              'Tesseract 5.5.4 / Leptonica 1.87.0 / zlib 1.4 / tessdata_fast eng.',
              'Pinned source locations and checksums: sources.json.',
              'Rust dependency versions: Cargo.lock.','']
    for path in sorted((root/'licenses').iterdir()):
        if path.is_file():sections += [f'## {path.name}','',path.read_text(), '']
    data=json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1','--manifest-path',str(root/'Cargo.toml')],cwd=root))
    grouped={}
    for package in sorted(data['packages'],key=lambda p:(p['name'],p['version'])):
        directory=Path(package['manifest_path']).parent
        files=[p for p in directory.iterdir() if p.is_file() and p.name.upper().startswith(('LICENSE','COPYING'))]
        sections += [f"- {package['name']} {package['version']}: {package.get('license') or 'see source license'}"]
        for path in sorted(files):
            text=path.read_text(errors='replace')
            digest=hashlib.sha256(text.encode()).hexdigest()
            entry=grouped.setdefault(digest,{'text':text,'packages':[]})
            entry['packages'].append(f"{package['name']} {package['version']} ({path.name})")
    sections += ['']
    for entry in grouped.values():
        sections += ['## '+', '.join(entry['packages']),'',entry['text'],'']
    payload='\n'.join(sections).encode()
    (root/'THIRD_PARTY_NOTICES.md').write_bytes(payload)
    def leb(number):
        result=bytearray()
        while number>127:result.append((number&127)|128);number>>=7
        return bytes(result)+bytes([number])
    name=b'sat:third-party-notices'
    body=leb(len(name))+name+payload
    section=b'\0'+leb(len(body))+body
    binary=component.read_bytes()
    if not binary.startswith(b'\0asm\x0d\0\x01\0'):raise ValueError('not a component')
    if not binary.endswith(section):component.write_bytes(binary+section)
