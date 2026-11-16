#!/usr/bin/env python3
"""Build and run the complete pinned upstream native Tesseract C++ suite.

Original tests/assertions stay unchanged. Native baseline results do not imply
that the production WASI image-profile component passes these internal API tests.
"""
import argparse,hashlib,json,os,shutil,subprocess,sys,time
from pathlib import Path
import xml.etree.ElementTree as ET

HERE=Path(__file__).resolve().parent
PIN='7e548e6eff172f38f97664afe7f7ce40db36c8d6'
ASSETS_PIN='fce038c79cbab4c45bac56b6f8d4e6bc6a9d9cfd'

def run(command,log,**kwargs):
    with log.open('a') as stream:
        stream.write('$ '+ ' '.join(map(str,command))+'\n');stream.flush()
        result=subprocess.run(list(map(str,command)),stdout=stream,stderr=subprocess.STDOUT,**kwargs)
    if result.returncode:
        with log.open('rb') as stream:
            stream.seek(max(0,log.stat().st_size-16384))
            print('\n'.join(stream.read(16384).decode(errors='replace').splitlines()[-45:]),flush=True)
    return result.returncode

def revision(directory):return subprocess.check_output(['git','-C',str(directory),'rev-parse','HEAD'],text=True).strip()

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--init',action='store_true',help='Initialize the pinned upstream and nested Git submodules')
    parser.add_argument('--jobs',type=int,default=6)
    parser.add_argument('--timeout',type=int,default=600,help='Per upstream test executable, seconds')
    parser.add_argument('--output',type=Path,default=HERE/'target/upstream-suite')
    parser.add_argument('--inventory-only',action='store_true')
    parser.add_argument('--include-disabled',action='store_true',help='Opt into upstream DISABLED_ cases; not part of the standard baseline')
    args=parser.parse_args()
    upstream=HERE/'upstream/tesseract'
    work=args.output.resolve();work.mkdir(parents=True,exist_ok=True)
    log=work/('inventory.log' if args.inventory_only else 'build.log');log.write_text('')
    report={'upstream_commit':PIN,'assets_commit':ASSETS_PIN,'backend':'native-full-upstream',
            'production_wasi_coverage':False,'stage':'setup','status':'incomplete'}
    def save(): (work/('inventory-summary.json' if args.inventory_only else 'summary.json')).write_text(json.dumps(report,indent=2)+'\n')
    if not args.inventory_only:save()
    if args.init:
        code=run(['git','-C',HERE.parent.parent,'submodule','update','--init','--recursive','--depth','1',
                  'external_tools/tesseract/upstream/tesseract'],log)
        if code:report.update(status='setup_failed',exit=code);save();return code
    if not (upstream/'CMakeLists.txt').exists():raise SystemExit('Run with --init to initialize the upstream submodules')
    assert revision(upstream)==PIN,'upstream checkout differs from pinned release'
    nested=subprocess.check_output(['git','-C',str(upstream),'submodule','status','--recursive'],text=True)
    assert all(line.startswith(' ') for line in nested.splitlines()),'nested submodules must match their gitlinks'
    report['nested_submodules']=nested.splitlines()
    inventory=sorted(p.stem for p in (upstream/'unittest').glob('*.cc'))
    report['source_test_executables']=inventory
    (work/'inventory.json').write_text(json.dumps({'test_executables':inventory,'count':len(inventory)},indent=2)+'\n')
    if args.inventory_only:
        report.update(stage='inventory',status='available')
        (work/'inventory-summary.json').write_text(json.dumps(report,indent=2)+'\n')
        print(f'{len(inventory)} original C++ test files available');return 0
    assets=work/'assets'
    if not (assets/'.git').exists():
        assets.mkdir(exist_ok=True)
        for command in [ ['git','init',assets],['git','-C',assets,'remote','add','origin','https://github.com/egorpugin/tessdata.git'],
                         ['git','-C',assets,'fetch','--depth','1','origin',ASSETS_PIN],['git','-C',assets,'checkout','--detach','FETCH_HEAD'] ]:
            code=run(command,log)
            if code:report.update(status='setup_failed',exit=code);save();return code
    assert revision(assets)==ASSETS_PIN
    # A build snapshot keeps generated fonts, tmp files and training artifacts
    # out of the tracked upstream checkout and its nested test-data submodule.
    source=work/'source'
    if not source.exists():
        print('Snapshotting unchanged upstream sources and test data...',flush=True)
        shutil.copytree(upstream,source,ignore=shutil.ignore_patterns('.git'))
    for name in ['tessdata','tessdata_best','tessdata_fast','langdata_lstm']:
        assert (assets/name).exists(),f'upstream assets missing {name}'
        if not (work/name).exists(): shutil.copytree(assets/name,work/name)
    # Unlike TESSDATA_DIR, upstream CMake's LANGDATA_DIR resolves inside the
    # source tree. Autotools CI places it beside the source tree instead.
    if not (source/'langdata_lstm').exists():shutil.copytree(assets/'langdata_lstm',source/'langdata_lstm')
    for font in (assets/'fonts').iterdir():
        if font.is_file():shutil.copy2(font,source/'test/testing'/font.name)
    # Verify every tracked upstream source/test file in the snapshot. Additional
    # fixture fonts are preparation; no test source/assertion is edited.
    tracked=subprocess.check_output(['git','-C',str(upstream),'ls-files','-z']).decode().split('\0')
    hashes={}
    for name in tracked:
        original=upstream/name
        if not name or not original.is_file():continue
        expected=hashlib.sha256(original.read_bytes()).hexdigest()
        assert hashlib.sha256((source/name).read_bytes()).hexdigest()==expected,name
        if name.startswith('unittest/'):hashes[name]=expected
    report['unchanged_test_sources']=hashes
    report.update(stage='build');save()
    prefix=work/'install';build=work/'build'
    lept_source=HERE/'target/sources/leptonica'
    if not lept_source.exists():
        from build import sources
        sources()
    common=['-G','Ninja','-DCMAKE_BUILD_TYPE=Release','-DBUILD_SHARED_LIBS=OFF',f'-DCMAKE_INSTALL_PREFIX={prefix}',
            '-DCMAKE_INSTALL_LIBDIR=lib','-DCMAKE_POLICY_VERSION_MINIMUM=3.5']
    lept=work/'leptonica'
    commands=[ ['cmake','-S',lept_source,'-B',lept,*common,'-DBUILD_PROG=OFF','-DSW_BUILD=OFF',
                '-DENABLE_PNG=ON','-DENABLE_JPEG=ON','-DENABLE_TIFF=ON','-DENABLE_GIF=OFF','-DENABLE_WEBP=OFF','-DENABLE_OPENJPEG=OFF'],
               ['cmake','--build',lept,'--parallel',args.jobs],['cmake','--install',lept],
               ['cmake','-S',source,'-B',build,*common,f'-DCMAKE_PREFIX_PATH={prefix}',
                '-DBUILD_TESTS=ON','-DBUILD_TRAINING_TOOLS=ON','-DDISABLED_LEGACY_ENGINE=OFF','-DUSE_SYSTEM_GTEST=OFF',
                '-DUSE_SYSTEM_ICU=ON','-DDISABLE_ARCHIVE=ON','-DDISABLE_CURL=ON','-DOPENMP_BUILD=OFF',
                '-DGRAPHICS_DISABLED=ON','-DENABLE_LTO=OFF','-DENABLE_CCACHE=OFF','-DENABLE_PRECOMPILED_HEADERS=OFF'],
               ['cmake','--build',build,'--parallel',args.jobs] ]
    for command in commands:
        print('Building full upstream suite:',command[:3],flush=True)
        code=run(command,log)
        if code:report.update(status='build_failed',exit=code);save();return code
        if command[:3]==['cmake','-S',source]:
            # Upstream visits unittest before src/training discovers Pango.
            # A second configure exposes those cached detections to all tests.
            code=run(command,log)
            if code:report.update(status='build_failed',exit=code);save();return code
    listing=json.loads(subprocess.check_output(['ctest','--test-dir',str(build),'--show-only=json-v1'],text=True))
    actual=sorted(test['name'] for test in listing['tests'])
    report['configured_test_executables']=actual
    assert actual==inventory,{'missing':sorted(set(inventory)-set(actual)),'unexpected':sorted(set(actual)-set(inventory))}
    reports=work/'gtest';reports.mkdir(exist_ok=True)
    for path in reports.glob('*.xml'):path.unlink()
    env=dict(os.environ,GTEST_OUTPUT='xml:'+str(reports)+'/',OMP_THREAD_LIMIT='1')
    for name in ['GTEST_FILTER','GTEST_TOTAL_SHARDS','GTEST_SHARD_INDEX','GTEST_SHARD_STATUS_FILE','GTEST_REPEAT','GTEST_SHUFFLE']:
        env.pop(name,None)
    env['GTEST_ALSO_RUN_DISABLED_TESTS']='1' if args.include_disabled else '0'
    report['include_disabled']=args.include_disabled
    report.update(stage='tests');save()
    print(f'Running all {len(actual)} unchanged upstream test executables...',flush=True)
    started=time.monotonic()
    code=run(['ctest','--test-dir',build,'--parallel','1','--timeout',args.timeout,'--output-on-failure',
              '--test-output-size-passed','1024','--test-output-size-failed','65536','--test-output-truncation','tail',
              '--output-junit',work/'ctest.xml'],work/'ctest.log',env=env)
    cases=[]
    for path in sorted(reports.glob('*.xml')):
        tree=ET.parse(path).getroot()
        for suite in tree.findall('testsuite'):
            for case in suite.findall('testcase'):
                status='failed' if case.find('failure') is not None else 'skipped' if case.find('skipped') is not None else 'disabled' if case.get('status')=='notrun' else 'passed'
                cases.append({'binary':path.stem,'suite':suite.get('name'),'name':case.get('name'),'status':status,'time_seconds':case.get('time')})
    report.update(stage='complete',status='passed' if code==0 else 'failed',exit=code,
                  elapsed_seconds=time.monotonic()-started,test_cases=cases,
                  counts={state:sum(c['status']==state for c in cases) for state in ['passed','failed','skipped','disabled']})
    ctest=ET.parse(work/'ctest.xml').getroot()
    report['test_executables']=[{'name':c.get('name'),'status':'failed' if c.find('failure') is not None else 'skipped' if c.find('skipped') is not None else 'passed','time_seconds':c.get('time')} for c in ctest.findall('testcase')]
    save()
    print(json.dumps({k:report[k] for k in ['status','counts','elapsed_seconds']},indent=2),flush=True)
    return code

if __name__=='__main__':sys.exit(main())
