use std::{env, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let build = env::var_os("FFMPEG_BUILD_DIR").map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/wasi"));
    let prefix = env::var_os("FFMPEG_ZLIB_PREFIX").map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/wasi-zlib/install"));
    let sdk = PathBuf::from(env::var_os("WASI_SDK_PATH").expect("set WASI_SDK_PATH"));
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let probe = env::var_os("CARGO_FEATURE_PROBE").is_some();
    let mut objects = vec!["cmdutils", "opt_common"];
    if probe { objects.push("ffprobe"); }
    else { objects.extend(["ffmpeg", "ffmpeg_filter", "ffmpeg_hw", "ffmpeg_mux", "ffmpeg_opt"]); }
    let mut ar = Command::new(sdk.join("bin/llvm-ar"));
    ar.arg("crs").arg(out.join("libmedia_cli.a"));
    for object in objects {
        let path = build.join(format!("fftools/{object}.o"));
        println!("cargo:rerun-if-changed={}", path.display());
        ar.arg(path);
    }
    assert!(ar.status().unwrap().success(), "archive upstream CLI objects");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=media_cli");
    for lib in ["avfilter", "avformat", "avcodec", "swresample", "swscale", "avutil"] {
        println!("cargo:rustc-link-search=native={}", build.join(format!("lib{lib}")).display());
        println!("cargo:rustc-link-lib=static={lib}");
        println!("cargo:rerun-if-changed={}", build.join(format!("lib{lib}/lib{lib}.a")).display());
    }
    println!("cargo:rustc-link-search=native={}", prefix.join("lib").display());
    println!("cargo:rustc-link-search=native={}", sdk.join("share/wasi-sysroot/lib/wasm32-wasip2").display());
    for lib in ["z", "wasi-emulated-signal", "wasi-emulated-mman", "wasi-emulated-process-clocks"] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    for var in ["WASI_SDK_PATH", "FFMPEG_BUILD_DIR", "FFMPEG_ZLIB_PREFIX"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
}
