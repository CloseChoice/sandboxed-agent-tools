"""Exercise the built WASI extractor, including capability and resource boundaries."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import zlib

ROOT = Path(__file__).resolve().parent


def pdf(page_contents, *, compressed=False, encrypted=False):
    """Generate real multi-page PDF fixtures without a native PDF dependency."""
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>', b'',
               b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>']
    kids = []
    for content in page_contents:
        page_id = len(objects) + 1
        stream_id = page_id + 1
        kids.append(f'{page_id} 0 R')
        objects.append(f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {stream_id} 0 R >>'.encode())
        if compressed:
            content = zlib.compress(content)
        filter_entry = b' /Filter /FlateDecode' if compressed else b''
        objects.append(b'<< /Length ' + str(len(content)).encode() + filter_entry + b' >>\nstream\n' + content + b'\nendstream')
    objects[1] = f'<< /Type /Pages /Kids [{" ".join(kids)}] /Count {len(kids)} >>'.encode()
    data = bytearray(b'%PDF-1.5\n%\xe2\xe3\xcf\xd3\n')
    offsets = [0]
    for number, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f'{number} 0 obj\n'.encode() + obj + b'\nendobj\n')
    xref = len(data)
    data.extend(f'xref\n0 {len(offsets)}\n0000000000 65535 f \n'.encode())
    for offset in offsets[1:]:
        data.extend(f'{offset:010d} 00000 n \n'.encode())
    encryption = ' /Encrypt << /Filter /Standard /V 99 >>' if encrypted else ''
    data.extend(f'trailer\n<< /Size {len(offsets)} /Root 1 0 R{encryption} >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return bytes(data)


def text_page(text):
    encoded = text.encode('cp1252').replace(b'\\', b'\\\\').replace(b'(', b'\\(').replace(b')', b'\\)')
    return b'BT /F1 12 Tf 72 720 Td (' + encoded + b') Tj ET'


def check(sat):
    with tempfile.TemporaryDirectory(prefix='sat-pdf-text-') as temporary:
        base = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(base / 'registry'), SAT_TEST_SECRET='unrelated-host-secret')
        calls = 0

        def invoke(*args, ok=True, data=b''):
            nonlocal calls
            result = subprocess.run([str(sat), *map(str, args)], cwd=base, env=env,
                                    input=data, capture_output=True, timeout=180)
            assert (result.returncode == 0) == ok, (args, result.stdout, result.stderr)
            assert b'unrelated-host-secret' not in result.stdout + result.stderr
            calls += 1
            return result

        invoke('register', ROOT / 'dist/pdf-text.tar', data=b'[]\n' * 5 + b'unset\n')
        assert b'max-pages' in invoke('pdf-text', '--help').stdout
        source = base / 'report.pdf'
        original = pdf([text_page('First page needle'), text_page('Second page café €'), b''])
        source.write_bytes(original)

        def extract(path=source, *options, ok=True, read=None):
            grant = path if read is None else read
            return invoke('pdf-text', path, *options, '--', '--read', grant, ok=ok)

        result = extract().stdout.decode()
        assert '--- Page 1 of 3 ---' in result and 'First page needle' in result
        assert '--- Page 2 of 3 ---' in result and 'café €' in result
        assert 'No extractable embedded text' in result
        assert str(base) not in result
        structured = json.loads(extract(source, '--format', 'json').stdout)
        assert structured['total_pages'] == 3 and structured['next_page'] is None
        assert [p['number'] for p in structured['pages']] == [1, 2, 3]
        assert structured['pages'][2]['text'].strip() == ''
        assert not structured['truncated']

        preview = json.loads(extract(source, '--format', 'json', '--max-pages', '1').stdout)
        assert len(preview['pages']) == 1 and preview['next_page'] == 2 and preview['truncated']
        assert 'Second page' not in json.dumps(preview)
        resumed = json.loads(extract(source, '--format', 'json', '--start-page', '2', '--max-pages', '1').stdout)
        assert resumed['pages'][0]['number'] == 2 and 'Second page' in resumed['pages'][0]['text']
        clipped = json.loads(extract(source, '--format', 'json', '--max-bytes', '5').stdout)
        assert clipped['pages'][0]['text'] == 'First'
        assert clipped['pages'][0]['truncated'] and clipped['next_page'] == 1

        unicode_pdf = base / 'unicode.pdf'
        unicode_pdf.write_bytes(pdf([text_page('éé')]))
        clipped_unicode = json.loads(extract(unicode_pdf, '--format', 'json', '--max-bytes', '1').stdout)
        assert clipped_unicode['pages'][0]['text'] == '' and clipped_unicode['pages'][0]['truncated']
        assert len(json.loads(extract(unicode_pdf, '--format', 'json', '--max-bytes', '2').stdout)['pages'][0]['text'].encode()) == 2

        for option, value in [('start-page', '0'), ('start-page', '4'), ('start-page', '4294967295'),
                              ('max-pages', '0'), ('max-pages', '201'), ('max-bytes', '0'),
                              ('max-bytes', '1048577'), ('format', 'xml')]:
            extract(source, '--' + option, value, ok=False)

        # A path argument is never a filesystem grant, nor is a sibling implied.
        denied = invoke('pdf-text', source, ok=False)
        assert b'First page needle' not in denied.stdout
        outside = base / 'secret.pdf'
        outside.write_bytes(pdf([text_page('outside-only secret')]))
        denied = extract(outside, ok=False, read=source)
        assert b'outside-only secret' not in denied.stdout
        if os.name != 'nt':
            link = base / 'escape.pdf'
            link.symlink_to(outside)
            extract(link, ok=False, read=source)
            folder = base / 'allowed'
            folder.mkdir()
            (folder / 'escape.pdf').symlink_to(outside)
            extract(folder / 'escape.pdf', ok=False, read=folder)
        protected = base / 'registry' / 'private.pdf'
        protected.write_bytes(pdf([text_page('protected-registry-content')]))
        denied = extract(protected, ok=False, read=base)
        assert b'protected-registry-content' not in denied.stdout

        malformed = base / 'malformed.pdf'
        malformed.write_bytes(b'not a PDF')
        extract(malformed, ok=False)
        malformed.write_bytes(pdf([text_page('encrypted text')], encrypted=True))
        extract(malformed, ok=False)
        malformed.write_bytes(pdf([]))
        extract(malformed, ok=False)
        malformed.write_bytes(pdf([b' ' * (2 * 1024 * 1024 + 1)], compressed=True))
        extract(malformed, ok=False)
        malformed.write_bytes(b'x' * (16 * 1024 * 1024 + 1))
        assert b'16 MiB' in extract(malformed, ok=False).stderr

        compressed = base / 'compressed.pdf'
        compressed.write_bytes(pdf([text_page('Compressed page')], compressed=True))
        assert b'Compressed page' in extract(compressed).stdout
        # The guest never needs write authority; every input remains unchanged.
        assert hashlib.sha256(source.read_bytes()).digest() == hashlib.sha256(original).digest()
        assert not (base / 'output.txt').exists()
        print(f'pdf-text: {calls} checks passed (page text, Unicode, previews, limits, malformed input and filesystem confinement)')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sat', type=Path, default=Path.home() / '.codex/skills/sat/scripts/run_sat.py')
    parser.add_argument('--skip-build', action='store_true')
    args = parser.parse_args()
    if not args.skip_build:
        subprocess.run(['bash', str(ROOT / 'package.sh')], check=True)
    check(args.sat.resolve())
