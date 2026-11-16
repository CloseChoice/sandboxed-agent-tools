"""Trusted cross-build orchestration for the upstream FFmpeg CLI extraction profile."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parent
TARGET = ROOT / 'target'
VERSION = '5.1.10'
SOURCES = {
    'ffmpeg': {'version': VERSION, 'url': f'https://ffmpeg.org/releases/ffmpeg-{VERSION}.tar.xz',
               'sha256': '392306d6fc45dab0e9e0ea55381e071842e83a2fb31d320aeda40477a7766293'},
    'zlib': {'version': '1.4', 'url': 'https://codeload.github.com/madler/zlib/tar.gz/v1.4',
             'sha256': '4331355048e286908260fdbeaa0b8cf75ceaf2907c75855a093a6487c7032aaa'},
}
PROFILE = [
    '--disable-asm', '--disable-inline-asm', '--disable-x86asm', '--disable-runtime-cpudetect',
    '--disable-pthreads', '--disable-w32threads', '--disable-os2threads',
    '--disable-network', '--disable-autodetect', '--disable-doc', '--disable-debug',
    '--disable-ffplay', '--disable-avdevice', '--disable-postproc', '--disable-iconv',
    '--disable-everything', '--enable-zlib',
    '--enable-protocol=file,pipe',
    '--enable-demuxer=mov,matroska,avi,mp3,wav,flac,ogg,image2,aac',
    '--enable-muxer=image2,wav,flac,null',
    '--enable-decoder=h264,hevc,mpeg4,mjpeg,png,bmp,vp8,vp9,aac,mp3,flac,vorbis,opus,pcm_s16le,pcm_s16be,pcm_f32le,pcm_s24le,pcm_s32le,rawvideo',
    '--enable-encoder=png,bmp,pcm_s16le,pcm_f32le,flac',
    '--enable-parser=h264,hevc,mpeg4video,mjpeg,png,vp8,vp9,aac,mpegaudio,flac,vorbis,opus',
    '--enable-filter=scale,format,aformat,aresample,anull,null,trim,atrim,fps,select,asetpts,setpts,transpose,crop,volume',
]
CLI_OBJECTS = ['cmdutils', 'opt_common', 'ffmpeg', 'ffmpeg_filter', 'ffmpeg_hw', 'ffmpeg_mux', 'ffmpeg_opt', 'ffprobe']
LIBRARIES = ['avformat', 'avcodec', 'avfilter', 'swresample', 'swscale', 'avutil']


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source(name):
    spec = SOURCES[name]
    cache = TARGET / 'downloads'
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / (name + '.tar')
    if not archive.exists():
        temporary = archive.with_suffix('.partial')
        urllib.request.urlretrieve(spec['url'], temporary)
        if digest(temporary) != spec['sha256']:
            temporary.unlink()
            raise RuntimeError(f'{name}: source checksum mismatch')
        temporary.rename(archive)
    if digest(archive) != spec['sha256']:
        raise RuntimeError(f'{name}: source checksum mismatch')
    destination = TARGET / 'sources' / name
    if not destination.exists():
        staging = TARGET / 'sources' / (name + '-unpack')
        staging.mkdir(parents=True, exist_ok=True)
        with tarfile.open(archive) as package:
            package.extractall(staging, filter='data')
        entries = list(staging.iterdir())
        if len(entries) != 1 or not entries[0].is_dir():
            raise RuntimeError(f'{name}: expected one source directory')
        entries[0].rename(destination)
        staging.rmdir()
    if name == 'ffmpeg':
        # The selected workflows do not require implicit temporary files. WASI
        # has no tempnam; report unsupported instead of fabricating host access.
        path = destination / 'libavutil/file_open.c'
        text = path.read_text()
        if '#ifdef __wasi__\n    *filename = NULL;' not in text:
            start = text.index('{', text.index('int avpriv_tempfile(')) + 1
            end = text.index('\n}', start)
            path.write_text(text[:start] + '\n#ifdef __wasi__\n    *filename = NULL;\n    return AVERROR(ENOSYS);\n#else' + text[start:end] + '\n#endif' + text[end:])
    return destination


def commands(label, entries, cwd=None, env=None):
    log = TARGET / (label + '.log')
    print(f'{label}: {log}', flush=True)
    with log.open('w') as stream:
        for command in entries:
            stream.write('$ ' + repr(list(map(str, command))) + '\n')
            stream.flush()
            result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT)
            if result.returncode:
                stream.flush()
                print(log.read_text()[-10000:], flush=True)
                raise RuntimeError(f'{label} failed ({result.returncode}); see {log}')


def build(native=False):
    TARGET.mkdir(exist_ok=True)
    ffmpeg, zlib = source('ffmpeg'), source('zlib')
    label = 'native' if native else 'wasi'
    sdk = Path(os.environ.get('WASI_SDK_PATH', ROOT.parents[1] / '.toolchain/wasi-sdk-34.0-x86_64-linux')).resolve()
    prefix = TARGET / (label + '-zlib/install')
    zbuild = TARGET / (label + '-zlib/build')
    zoptions = ['-G', 'Ninja', '-DCMAKE_BUILD_TYPE=Release', '-DZLIB_BUILD_SHARED=OFF', '-DZLIB_BUILD_STATIC=ON',
                '-DZLIB_BUILD_TESTING=OFF', f'-DCMAKE_INSTALL_PREFIX={prefix}', '-DCMAKE_INSTALL_LIBDIR=lib']
    if not native:
        zoptions += [f'-DCMAKE_TOOLCHAIN_FILE={sdk}/share/cmake/wasi-sdk-p2.cmake']
    commands(label + '-zlib', [
        ['cmake', '-S', zlib, '-B', zbuild, *zoptions],
        ['cmake', '--build', zbuild, '--parallel', '6'],
        ['cmake', '--install', zbuild],
    ])
    directory = TARGET / label
    directory.mkdir(exist_ok=True)
    configure = [str(ffmpeg / 'configure'), *PROFILE]
    if native:
        configure += [f'--extra-cflags=-I{prefix}/include', f'--extra-ldflags=-L{prefix}/lib']
    else:
        flags = '--target=wasm32-wasip2 -D_WASI_EMULATED_SIGNAL -D_WASI_EMULATED_MMAN -D_WASI_EMULATED_PROCESS_CLOCKS'
        configure += [
            '--enable-cross-compile', '--target-os=none', '--arch=wasm32',
            *[f'--{key}={sdk}/bin/{tool}' for key, tool in [('cc', 'clang'), ('ld', 'clang'), ('ar', 'llvm-ar'), ('nm', 'llvm-nm'), ('ranlib', 'llvm-ranlib')]],
            f'--extra-cflags={flags} -I{prefix}/include',
            f'--extra-ldflags=--target=wasm32-wasip2 -L{prefix}/lib -lwasi-emulated-signal -lwasi-emulated-mman -lwasi-emulated-process-clocks',
            '--extra-libs=-lm',
        ]
    configuration = json.dumps(configure)
    stamp = directory / '.sat-configure.json'
    if not stamp.exists() or stamp.read_text() != configuration:
        commands(label + '-configure', [configure], cwd=directory)
        stamp.write_text(configuration)
    make = ['make', '-j6']
    if native:
        make += ['ffmpeg', 'ffprobe']
    else:
        make += ['CFLAGS-ffmpeg=-Dmain=sat_media_main -Wno-error=missing-prototypes',
                 'CFLAGS-ffprobe=-Dmain=sat_media_main -Wno-error=missing-prototypes',
                 *[f'lib{lib}/lib{lib}.a' for lib in LIBRARIES],
                 *[f'fftools/{obj}.o' for obj in CLI_OBJECTS]]
    commands(label + '-make', [make], cwd=directory)
    if native:
        return
    env = dict(os.environ, WASI_SDK_PATH=str(sdk))
    for tool, features in [('ffmpeg', []), ('ffprobe', ['--features', 'probe'])]:
        commands(tool + '-component', [['cargo', 'build', '--locked', '--release', '--target', 'wasm32-wasip2', *features]], cwd=ROOT, env=env)
        shutil.copy2(TARGET / 'wasm32-wasip2/release/sat_ffmpeg_guest.wasm', TARGET / (tool + '.wasm'))
    (TARGET / 'build-manifest.json').write_text(json.dumps({
        'sources': SOURCES, 'configuration': configure, 'sdk': str(sdk),
        'components': {name: {'bytes': (TARGET / (name + '.wasm')).stat().st_size,
                              'sha256': digest(TARGET / (name + '.wasm'))} for name in ['ffmpeg', 'ffprobe']},
    }, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native', action='store_true', help='build a matching upstream native reference')
    build(parser.parse_args().native)
