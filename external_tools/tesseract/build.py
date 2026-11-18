"""Trusted builder: verified source archives, native CMake/SDK, then a WASI component."""
from pathlib import Path
import argparse,hashlib,json,os,subprocess,tarfile,urllib.request
from build_libs import build as build_libraries

ROOT=Path(__file__).resolve().parent
TARGET=ROOT/'target'

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()

def sources():
    TARGET.mkdir(exist_ok=True)
    records=json.loads((ROOT/'sources.json').read_text())
    for name,record in records.items():
        archive=TARGET/('eng.traineddata' if name=='eng' else f'{name}.tar.gz')
        if not archive.exists():
            pending=archive.with_suffix('.download')
            try:
                with urllib.request.urlopen(record['url'],timeout=120) as response:
                    with pending.open('wb') as stream:
                        while data:=response.read(1024*1024): stream.write(data)
                assert sha(pending)==record['sha256'],f'{name}: download digest mismatch'
                pending.replace(archive)
            finally: pending.unlink(missing_ok=True)
        if sha(archive)!=record['sha256']: raise ValueError(f'{name}: cached source digest mismatch')
        if name=='eng': continue
        destination=TARGET/'sources'/name
        marker=destination/'.sat-source-sha256'
        if destination.exists():
            if not marker.exists() or marker.read_text()!=record['sha256']:
                # Sources from development or a different lock are replaced in full.
                import shutil
                shutil.rmtree(destination)
        if not destination.exists():
            destination.mkdir(parents=True)
            with tarfile.open(archive) as tf:
                prefix=tf.getmembers()[0].name.split('/')[0]+'/'
                for member in tf.getmembers():
                    if not member.name.startswith(prefix): continue
                    member.name=member.name[len(prefix):]
                    if member.name: tf.extract(member,destination,filter='data')
            marker.write_text(record['sha256'])

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native',action='store_true',help='Build the matching native reference binary and upstream CLI')
    args=parser.parse_args()
    sources()
    if not args.native:
        fallback=ROOT.parent.parent/'.toolchain/wasi-sdk-34.0-x86_64-linux'
        sdk=Path(os.environ.get('WASI_SDK_PATH',fallback)).resolve()
        if not (sdk/'bin/clang++').exists(): raise SystemExit('Set WASI_SDK_PATH to WASI SDK 34')
        os.environ['WASI_SDK_PATH']=str(sdk)
    # Let CMake and Cargo validate their incremental builds; downloaded inputs
    # are independently checked against the lock on every invocation.
    build_libraries(native=args.native)
    command=['cargo','build','--locked','--release','--manifest-path',str(ROOT/'Cargo.toml'),'--target-dir',str(TARGET)]
    command += ['--bin','ocr-native'] if args.native else ['--target','wasm32-wasip2','--lib']
    subprocess.run(command,check=True,cwd=ROOT)
    output=TARGET/('release/ocr-native' if args.native else 'wasm32-wasip2/release/sat_tesseract_guest.wasm')
    if not args.native:
        from notices import attach
        attach(ROOT,output)
    print(f'{output}: {output.stat().st_size} bytes; SHA256 {sha(output)}',flush=True)

if __name__=='__main__': main()
