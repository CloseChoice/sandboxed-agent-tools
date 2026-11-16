#!/usr/bin/env python3
"""Compile original upstream core tests into isolated test-only SAT components.

Full source inventory is retained. Tests outside the image runtime profile are
reported separately, and build/runtime/assertion failures are never counted as
passes. No original test source or assertion is changed.
"""
import argparse,hashlib,io,json,os,re,shutil,subprocess,sys,tarfile,time,urllib.request
from pathlib import Path
import xml.etree.ElementTree as ET
from upstream_suite import PIN,revision,run

HERE=Path(__file__).resolve().parent
WRAPPER=Path.home()/'.codex/skills/sat/scripts/run_sat.py'

def cmake_list(text,name):
    match=re.search(r'set\('+re.escape(name)+r'\s+(.*?)\)',text,re.S)
    assert match,name
    return {Path(s).stem for s in match.group(1).split()}

def cases(path):
    tree=ET.parse(path).getroot();result=[]
    for suite in tree.findall('testsuite'):
        for case in suite.findall('testcase'):
            status='failed' if case.find('failure') is not None else 'skipped' if case.find('skipped') is not None else 'disabled' if case.get('status')=='notrun' else 'passed'
            result.append({'suite':suite.get('name'),'name':case.get('name'),'status':status,'seconds':case.get('time')})
    return result

def compare_native(report):
    native_summary=HERE/'target/upstream-suite/summary.json'
    if not native_summary.exists():return
    native=json.loads(native_summary.read_text())
    native_cases={(c['binary'],c['suite'],c['name']):c['status'] for c in native.get('test_cases',[])}
    matched=[]
    for record in report['test_executables']:
        expected=native.get('unchanged_test_sources',{}).get('unittest/'+record['name']+'.cc')
        if expected:assert expected==record['source_sha256'],'native/WASI test-source mismatch'
        for case in record.get('test_cases',[]):
            key=(record['name'],case['suite'],case['name'])
            if key in native_cases:matched.append({'binary':record['name'],'suite':case['suite'],'name':case['name'],
                                                    'native':native_cases[key],'sat_wasi':case['status']})
    report['matched_native_cases']=matched
    report['comparison_counts']={}
    for case in matched:
        key=case['native']+'/'+case['sat_wasi']
        report['comparison_counts'][key]=report['comparison_counts'].get(key,0)+1

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sat',type=Path,required=True)
    parser.add_argument('--output',type=Path,default=HERE/'target/upstream-wasi')
    parser.add_argument('--only',nargs='+',help='Select individual upstream test executables within the core profile')
    parser.add_argument('--timeout',type=int,default=120)
    parser.add_argument('--include-exception-tests',action='store_true',help='Also attempt original tests whose tempdir helper requires unsupported C++ exceptions')
    parser.add_argument('--compare-only',action='store_true',help='Refresh paired results after a parallel native run, without rebuilding or rerunning tests')
    args=parser.parse_args()
    upstream=HERE/'upstream/tesseract'
    assert revision(upstream)==PIN
    subprocess.run(['git','-C',str(upstream),'diff','--exit-code','HEAD','--','unittest','src','include'],check=True,capture_output=True)
    work=args.output.resolve();work.mkdir(parents=True,exist_ok=True)
    if args.compare_only:
        report=json.loads((work/'summary.json').read_text());compare_native(report)
        (work/'summary.json').write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps(report.get('comparison_counts',{}),indent=2));return 0
    sdk=Path(os.environ.get('WASI_SDK_PATH',HERE.parent.parent/'.toolchain/wasi-sdk-34.0-x86_64-linux')).resolve()
    if not (sdk/'bin/clang++').exists():raise SystemExit('Set WASI_SDK_PATH to SDK 34')
    prefix=HERE/'target/wasi/install'
    if not (prefix/'lib/libtesseract.a').exists():raise SystemExit('Build the production WASI component first')
    original_gtest=upstream/'unittest/third_party/googletest'
    assert (original_gtest/'CMakeLists.txt').exists(),'initialize upstream nested submodules'
    harness=HERE/'upstream_harness'
    gtest_build=work/'gtest'
    setup=work/'setup.log';setup.write_text('')
    commands=[['cmake','-S',harness/'gtest','-B',gtest_build,'-G','Ninja',f'-DGTEST_SOURCE={original_gtest}',
               f'-DCMAKE_TOOLCHAIN_FILE={sdk}/share/cmake/wasi-sdk-p2.cmake','-DCMAKE_BUILD_TYPE=Release'],
              ['cmake','--build',gtest_build,'--parallel','2']]
    for command in commands:
        code=run(command,setup)
        if code:return code
    text=(upstream/'unittest/CMakeLists.txt').read_text()
    legacy=cmake_list(text,'LEGACY_TESTS');training=cmake_list(text,'TRAINING_TESTS');pango=cmake_list(text,'PANGO_TESTS')
    image_api={'apiexample_test','baseapi_test','baseapi_thread_test','capiexample_test','cleanapi_test','imagedata_test',
               'layout_test','loadlang_test','pageres_test','pagesegmode_test','plumbing_test','progress_test',
               'resultiterator_test','unicharset_load_test'}
    extra_training={'lang_model_test','lstmtrainer_test','fileio_test','recodebeam_test'}
    inventory=sorted(p.stem for p in (upstream/'unittest').glob('*.cc'))
    excluded={}
    for name in inventory:
        if name in pango:excluded[name]='Pango/font-dependent training API is not in the production WASI profile'
        elif name in legacy:excluded[name]='Legacy engine is disabled in the production WASI profile'
        elif name in training or name in extra_training:excluded[name]='Training/ICU libraries are not in the production WASI profile'
        elif name in image_api:excluded[name]='Original image/model/lifecycle/thread API needs test adapters beyond this core pass; production CLI has separate integration coverage'
        elif name=='tatweel_test':excluded[name]='Additional upstream Unicode test sources have not yet been added to the WASI harness'
        elif name in {'lstm_nullchar_test','lstm_recoder_test'} and not args.include_exception_tests:excluded[name]='Original util/tempdir.h uses C++ try/catch; the production WASI engine uses the no-exceptions runtime'
    selected=[name for name in inventory if name not in excluded]
    if args.only:
        assert set(args.only)<=set(selected),{'outside_core_profile':sorted(set(args.only)-set(selected))}
        for name in selected:
            if name not in args.only:excluded[name]='Not selected by --only'
        selected=[name for name in selected if name in args.only]
    langdata=work/'langdata';langdata.mkdir(exist_ok=True)
    if 'recodebeam_test' in selected:
        radical=langdata/'radical-stroke.txt'
        digest='9b441331169924e640233ae3abff9b4ac7019e33978a074c9d322f89afab1b89'
        if not radical.exists():
            url='https://raw.githubusercontent.com/egorpugin/tessdata/fce038c79cbab4c45bac56b6f8d4e6bc6a9d9cfd/langdata_lstm/radical-stroke.txt'
            with urllib.request.urlopen(url,timeout=60) as response:radical.write_bytes(response.read())
        assert hashlib.sha256(radical.read_bytes()).hexdigest()==digest,'language-data checksum mismatch'
    report={'upstream_commit':PIN,'backend':'sat-wasi-original-core-tests','full_upstream_suite':False,
            'production_library':str(prefix/'lib/libtesseract.a'),'source_test_files':len(inventory),
            'excluded':excluded,'test_executables':[],'status':'running',
            'gtest_configuration':'Single-threaded, no exceptions, no death tests or stream redirection; original source/assertions unchanged.'}
    def save():
        all_cases=[case for record in report['test_executables'] for case in record.get('test_cases',[])]
        report['counts']={state:sum(c['status']==state for c in all_cases) for state in ['passed','failed','skipped','disabled']}
        (work/'summary.json').write_text(json.dumps(report,indent=2)+'\n')
    save()
    guest=harness/'guest'
    env=dict(os.environ,WASI_SDK_PATH=str(sdk),TESSERACT_UPSTREAM=str(upstream),OCR_PREFIX=str(prefix),
             GTEST_SOURCE=str(original_gtest),GTEST_BUILD=str(gtest_build),CARGO_TARGET_DIR=str(work/'guest-build'),
             LANGDATA_DIR=str(langdata),
             SAT_BINARY=str(args.sat.resolve()),SAT_HOME=str(work/'registry'))
    sat_prefix=[sys.executable,str(WRAPPER),'--'] if WRAPPER.exists() else [str(args.sat.resolve())]
    package=work/'packages';package.mkdir(exist_ok=True)
    xml=work/'xml';xml.mkdir(exist_ok=True)
    temps=work/'tmp';temps.mkdir(exist_ok=True)
    component=work/'guest-build/wasm32-wasip2/release/sat_tesseract_upstream_test.wasm'
    for index,name in enumerate(selected):
        print(f'[{index+1}/{len(selected)}] original {name} -> SAT/WASI',flush=True)
        log=work/f'{name}.log';log.write_text('')
        tmp=temps/name;tmp.mkdir(exist_ok=True)
        env.update(UPSTREAM_TEST=name,SAT_TEST_TMPDIR=str(tmp))
        record={'name':name,'source_sha256':hashlib.sha256((upstream/'unittest'/f'{name}.cc').read_bytes()).hexdigest()}
        code=run(['cargo','build','--locked','--release','--target','wasm32-wasip2','--manifest-path',guest/'Cargo.toml'],log,env=env)
        if code:
            record.update(status='build_failed',exit=code);report['test_executables'].append(record);save();continue
        tool='tesseract-upstream-'+name.replace('_','-')
        metadata=f'name = "{tool}"\nversion = "0.1.0"\ndescription = "Test-only unchanged upstream {name}"\nentrypoint = "run"\ncommand_output = true\ncli_passthrough = true\n[[arguments]]\nname = "args"\nhelp = "Original GoogleTest flags"\n'
        files={'tool.wasm':component.read_bytes(),'tool.toml':metadata.encode(),'defaults.toml':(HERE/'defaults.toml').read_bytes()}
        archive=package/f'{name}.tar'
        with tarfile.open(archive,'w') as tf:
            for filename,data in files.items():
                info=tarfile.TarInfo(filename);info.size=len(data);info.mode=0o644;tf.addfile(info,io.BytesIO(data))
        code=run([*sat_prefix,'register','--force',archive],log,input=b'[]\n'*5+b'unset\n',env=env)
        if code:
            record.update(status='registration_failed',exit=code);report['test_executables'].append(record);save();continue
        output=xml/f'{name}.xml';output.unlink(missing_ok=True)
        started=time.monotonic()
        try:
            grants=['--read',upstream/'test','--write',xml,'--write',tmp]
            if name=='recodebeam_test':grants+=['--read',langdata]
            code=run([*sat_prefix,tool,'--gtest_color=no',f'--gtest_output=xml:{output}',
                      '--',*grants],log,env=env,timeout=args.timeout)
        except subprocess.TimeoutExpired:
            record.update(status='timeout',elapsed_seconds=time.monotonic()-started)
            report['test_executables'].append(record);save();continue
        record.update(exit=code,elapsed_seconds=time.monotonic()-started)
        if output.exists():record['test_cases']=cases(output)
        if not record.get('test_cases'):record['status']='runtime_failed_or_empty_suite'
        elif code:record['status']='tests_failed'
        else:record['status']='passed'
        report['test_executables'].append(record);save()
    report['status']='passed' if all(t['status']=='passed' for t in report['test_executables']) else 'failed'
    compare_native(report)
    save()
    print(json.dumps({'status':report['status'],'counts':report['counts'],'selected_executables':len(selected),'excluded':len(excluded)},indent=2),flush=True)
    return 0 if report['status']=='passed' else 1

if __name__=='__main__':sys.exit(main())
