"""Trusted native/WASI cross-build orchestration for pinned OCR libraries."""
from pathlib import Path
import argparse,os,subprocess,shutil

ROOT=Path(__file__).resolve().parent
TARGET=ROOT/'target'

def build(native=False):
    sdk=Path(os.environ.get('WASI_SDK_PATH',ROOT.parent.parent/'.toolchain/wasi-sdk-34.0-x86_64-linux')).resolve()
    label='native' if native else 'wasi'
    prefix=TARGET/label/'install'
    prefix.mkdir(parents=True,exist_ok=True)
    common=['-G','Ninja','-DCMAKE_BUILD_TYPE=Release','-DBUILD_SHARED_LIBS=OFF',f'-DCMAKE_INSTALL_PREFIX={prefix}',f'-DCMAKE_PREFIX_PATH={prefix}','-DCMAKE_INSTALL_LIBDIR=lib','-DCMAKE_POLICY_VERSION_MINIMUM=3.5']
    if native: common += ['-DCMAKE_POSITION_INDEPENDENT_CODE=ON','-DCMAKE_CXX_FLAGS=-DTESSERACT_DISABLE_DEBUG_FONTS']
    if not native:
        common += [f'-DCMAKE_TOOLCHAIN_FILE={sdk}/share/cmake/wasi-sdk-p2.cmake',f'-DCMAKE_FIND_ROOT_PATH={prefix}',
                   f'-DCMAKE_C_FLAGS=-D_WASI_EMULATED_MMAN -D_WASI_EMULATED_SIGNAL -D_WASI_EMULATED_PROCESS_CLOCKS -include {ROOT}/wasi_compat.h',
                   f'-DCMAKE_CXX_FLAGS=-fno-exceptions -DTESSERACT_DISABLE_DEBUG_FONTS -D_WASI_EMULATED_MMAN -D_WASI_EMULATED_SIGNAL -D_WASI_EMULATED_PROCESS_CLOCKS -include {ROOT}/wasi_compat.h',
                   '-DCMAKE_EXE_LINKER_FLAGS=-lwasi-emulated-mman -lwasi-emulated-signal -lwasi-emulated-process-clocks']
    options={
      'zlib':['-DZLIB_BUILD_TESTING=OFF','-DZLIB_BUILD_SHARED=OFF','-DZLIB_BUILD_STATIC=ON'],
      # Decode image formats in Rust: native codec setjmp/longjmp requires Wasm EH.
      'leptonica':['-DBUILD_PROG=OFF','-DSW_BUILD=OFF','-DENABLE_PNG=OFF','-DENABLE_JPEG=OFF','-DENABLE_GIF=OFF','-DENABLE_TIFF=OFF','-DENABLE_WEBP=OFF','-DENABLE_OPENJPEG=OFF'],
      'tesseract':['-DBUILD_TRAINING_TOOLS=OFF','-DBUILD_TESTS=OFF','-DGRAPHICS_DISABLED=ON','-DDISABLED_LEGACY_ENGINE=ON','-DOPENMP_BUILD=OFF','-DDISABLE_TIFF=ON','-DDISABLE_ARCHIVE=ON','-DDISABLE_CURL=ON','-DENABLE_NATIVE=OFF','-DENABLE_LTO=OFF','-DENABLE_PRECOMPILED_HEADERS=OFF','-DENABLE_CCACHE=OFF'],
    }
    if not native:
        # Leptonica was explicitly configured without TIFF; do not execute its probe.
        options['tesseract'] += ['-DLEPT_TIFF_RESULT=1']
    env=dict(os.environ,PKG_CONFIG_LIBDIR=str(prefix/'lib/pkgconfig'),PKG_CONFIG_PATH=str(prefix/'lib/pkgconfig'))
    for name,flags in options.items():
        directory=TARGET/label/name
        log=TARGET/f'{label}-{name}.log'
        print(f'Building {name} ({label}); log {log}',flush=True)
        with log.open('w') as stream:
            commands=[
                ['cmake','-S',str(TARGET/'sources'/name),'-B',str(directory),*common,*flags],
                ['cmake','--build',str(directory),'--parallel','6'],
                ['cmake','--install',str(directory)],
            ]
            if name=='tesseract' and not native:
                commands[1] += ['--target','libtesseract']
                commands.pop()
            for command in commands:
                result=subprocess.run(command,env=env,stdout=stream,stderr=subprocess.STDOUT)
                if result.returncode:
                    stream.flush()
                    print('\n'.join(log.read_text().splitlines()[-70:]),flush=True)
                    raise SystemExit(result.returncode)
        if name=='tesseract' and not native:
            shutil.copy2(directory/'libtesseract.a',prefix/'lib/libtesseract.a')
            shutil.copytree(TARGET/'sources/tesseract/include/tesseract',prefix/'include/tesseract',dirs_exist_ok=True)
            shutil.copy2(directory/'include/tesseract/version.h',prefix/'include/tesseract/version.h')

if __name__=='__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--native',action='store_true')
    build(parser.parse_args().native)
