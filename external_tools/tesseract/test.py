#!/usr/bin/env python3
"""Native/WASI OCR comparisons on pinned upstream-derived fixtures and grant checks.

This integration suite uses an original upstream image and transcription; it does
not claim to run the full upstream Tesseract unit/regression suite.
"""
import argparse,csv,hashlib,html,io,json,os,re,statistics,subprocess,sys,tempfile,time
from pathlib import Path

HERE=Path(__file__).resolve().parent
WRAPPER=Path.home()/'.codex/skills/sat/scripts/run_sat.py'

def normalize(text): return ' '.join(text.split())

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sat',type=Path,required=True)
    parser.add_argument('--native',type=Path,default=HERE/'target/release/ocr-native')
    parser.add_argument('--upstream-cli',type=Path,default=HERE/'target/native/install/bin/tesseract')
    parser.add_argument('--model',type=Path,default=HERE/'target/eng.traineddata')
    parser.add_argument('--output',type=Path)
    args=parser.parse_args()
    args.sat=args.sat.resolve(); args.native=args.native.resolve()
    args.upstream_cli=args.upstream_cli.resolve(); args.model=args.model.resolve()
    records=[]
    fixtures=HERE/'fixtures'
    manifest=json.loads((fixtures/'manifest.json').read_text())
    for name,record in manifest.items():
        assert hashlib.sha256((fixtures/name).read_bytes()).hexdigest()==record['sha256'],name
    gold=normalize((fixtures/'phototest.gold.txt').read_text())

    def native(flags,data=None):
        return subprocess.run([str(args.native),*map(str,flags)],input=data,capture_output=True,timeout=45)

    def sat(flags,grants=(),data=None,cwd=None):
        prefix=[sys.executable,str(WRAPPER),'--'] if WRAPPER.exists() else [str(args.sat)]
        env=dict(os.environ,SAT_BINARY=str(args.sat))
        return subprocess.run([*prefix,'tesseract',*map(str,flags),'--',*map(str,grants)],
                              input=data,capture_output=True,timeout=45,env=env,cwd=cwd)

    def record(name,kind='boundary/profile',**extra):
        records.append({'name':name,'status':'passed','kind':kind,**extra})

    # Independent native upstream CLI, unmodified apart from build configuration.
    baseline=subprocess.run([str(args.upstream_cli),str(fixtures/'phototest.pgm'),'stdout',
        '--tessdata-dir',str(args.model.parent),'-l','eng','--psm','3'],capture_output=True,timeout=45)
    assert baseline.returncode==0,baseline.stderr
    assert normalize(baseline.stdout.decode())==gold,baseline.stdout
    record('upstream-native-ground-truth','upstream fixture')

    for filename in ['phototest.png','phototest.jpg','phototest.pgm','phototest.bmp','phototest-alpha.png']:
        image=fixtures/filename
        for config in ['txt','tsv','hocr']:
            flags=[image,'stdout','--psm','3',config]
            reference=native(flags)
            actual=sat(flags,['--read',image])
            assert reference.returncode==actual.returncode==0,(filename,config,reference.stderr,actual.stderr)
            assert actual.stderr==b'',(filename,config,actual.stderr)
            a,b=reference.stdout.decode(),actual.stdout.decode()
            if config=='txt':
                assert a==b,(filename,a,b)
                assert normalize(b)==gold,(filename,b)
            elif config=='tsv':
                ar=list(csv.DictReader(io.StringIO(a),delimiter='\t'))
                br=list(csv.DictReader(io.StringIO(b),delimiter='\t'))
                assert len(ar)==len(br)
                for one,two in zip(ar,br):
                    assert one.keys()==two.keys()
                    for key in one:
                        if key=='conf': assert abs(float(one[key])-float(two[key]))<0.1,(filename,one,two)
                        else: assert one[key]==two[key],(filename,one,two)
                assert normalize(' '.join(row['text'] for row in br if row['level']=='5'))==gold
            else:
                assert '<html ' in b and 'ocrx_word' in b
                words=lambda text:normalize(html.unescape(re.sub('<[^>]*>',' ',text)))
                assert words(a)==words(b)==gold,(filename,words(a),words(b))
            record(f'{filename}/{config}','native/WASI',exact_bytes=reference.stdout==actual.stdout)

    image=fixtures/'phototest.png'
    actual=sat(['stdin','stdout','--psm','3'],data=image.read_bytes())
    assert actual.returncode==0 and normalize(actual.stdout.decode())==gold,actual.stderr
    record('stdin-no-filesystem-grants')
    actual=sat(['phototest.png','stdout'],['--read',image],cwd=fixtures)
    assert actual.returncode==0 and normalize(actual.stdout.decode())==gold,actual.stderr
    record('relative-input')
    actual=sat([image,'stdout','--model',args.model],['--read',image,'--read',args.model])
    assert actual.returncode==0 and normalize(actual.stdout.decode())==gold,actual.stderr
    record('explicit-model-with-read-grant')
    blank=fixtures/'blank.png'
    actual=sat([blank,'stdout'],['--read',blank])
    assert actual.returncode==0 and actual.stdout.strip()==b'',actual.stderr
    record('blank-page')

    with tempfile.TemporaryDirectory(prefix='sat-ocr-test-') as tmp:
        root=Path(tmp)
        bad=root/'invalid.png';bad.write_bytes(b'not an image')
        large=root/'large.pgm'
        with large.open('wb') as f:
            header=b'P5\n5000 5000\n255\n';f.write(header);f.truncate(len(header)+25_000_000)
        wide=root/'wide.pgm';wide.write_bytes(b'P5\n12001 1\n255\n'+b'\xff'*12001)
        invalid_model=root/'invalid.traineddata';invalid_model.write_bytes(b'not a model')
        oversized=root/'oversized.png'
        with oversized.open('wb') as f:f.truncate(32*1024*1024+1)
        output=root/'result.txt';output.write_bytes(b'keep me')
        failures=[
            ('no-read-grant',[image,'stdout'],[]),
            ('unrelated-read-grant',[image,'stdout'],['--read',blank]),
            ('invalid-image',[bad,'stdout'],['--read',bad]),
            ('pixel-limit',[large,'stdout'],['--read',large]),
            ('dimension-limit',[wide,'stdout'],['--read',wide]),
            ('encoded-input-limit',[oversized,'stdout'],['--read',oversized]),
            ('unsupported-language',[image,'stdout','-l','deu'],['--read',image]),
            ('missing-model-grant',[image,'stdout','--model',args.model],['--read',image]),
            ('invalid-model',[image,'stdout','--model',invalid_model],['--read',image,'--read',invalid_model]),
            ('external-model-limit',[image,'stdout','--model',oversized],['--read',image,'--read',oversized]),
            ('unsupported-osd',[image,'stdout','--psm','0'],['--read',image]),
            ('unsupported-legacy-engine',[image,'stdout','--oem','0'],['--read',image]),
            ('external-config-rejected',[image,'stdout','unknown-config'],['--read',image]),
            ('output-write-denied',[image,root/'result'],['--read',image,'--read',output]),
        ]
        for name,flags,grants in failures:
            actual=sat(flags,grants)
            assert actual.returncode!=0,(name,actual.stdout)
            assert not actual.stdout,(name,actual.stdout)
            if name in ['pixel-limit','dimension-limit','encoded-input-limit','external-model-limit']:
                assert b'limit' in actual.stderr.lower() or b'20 million pixels' in actual.stderr,(name,actual.stderr)
            assert output.read_bytes()==b'keep me',name
            record(name,exit=actual.returncode)
        actual=sat([image,root/'result'],['--read',image,'--write',output])
        assert actual.returncode==0,actual.stderr
        assert actual.stdout==b''
        assert normalize(output.read_text())==gold
        record('exact-output-file-grant')
        actual=sat([image,root/'new-result','tsv'],['--read',image,'--write',root])
        assert actual.returncode==0,actual.stderr
        assert (root/'new-result.tsv').read_text().startswith('level\tpage_num\t')
        record('new-output-with-directory-grant')
        assert set(p.name for p in root.iterdir())=={'invalid.png','large.pgm','wide.pgm','oversized.png','invalid.traineddata','result.txt','new-result.tsv'}
        record('no-unrequested-output-files')

    timings={'native_ms':[],'sat_wasi_ms':[]}
    for _ in range(3):
        started=time.perf_counter();result=native([image,'stdout']);timings['native_ms'].append(1000*(time.perf_counter()-started))
        assert result.returncode==0
        started=time.perf_counter();result=sat([image,'stdout'],['--read',image]);timings['sat_wasi_ms'].append(1000*(time.perf_counter()-started))
        assert result.returncode==0
    for name,values in list(timings.items()): timings['median_'+name]=statistics.median(values)
    report={'tesseract':'5.5.4','leptonica':'1.87.0','model':'tessdata_fast eng',
            'scope':'Our image-profile integration suite with original upstream fixture/ground truth; not upstream full test suite.',
            'passed':len(records),'tests':records,'timing':timings,
            'timing_notes':'Three warm-registry CLI calls per backend; includes startup/logging overhead. Native and WASI use matching OCR source/model; floating-point/SIMD implementations can differ.'}
    if args.output:
        args.output.parent.mkdir(parents=True,exist_ok=True)
        args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(f'{len(records)} checks passed; native {timings["median_native_ms"]:.1f} ms, SAT/WASI {timings["median_sat_wasi_ms"]:.1f} ms median.')

if __name__=='__main__':main()
