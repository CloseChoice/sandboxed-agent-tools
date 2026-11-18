use std::{env, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let wasi = env::var("TARGET").unwrap() == "wasm32-wasip2";
    let label = if wasi { "wasi" } else { "native" };
    let prefix = root.join(format!("target/{label}/install"));
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let sdk = env::var_os("WASI_SDK_PATH").map(PathBuf::from);
    let compiler = if wasi {
        sdk.as_ref().expect("set WASI_SDK_PATH").join("bin/clang++")
    } else {
        PathBuf::from("c++")
    };
    let archiver = if wasi {
        sdk.as_ref().unwrap().join("bin/llvm-ar")
    } else {
        PathBuf::from("ar")
    };
    let mut command = Command::new(compiler);
    command
        .args(["-std=c++20", "-O2", "-fPIC", "-fno-exceptions", "-c"])
        .arg(root.join("bridge.cpp"))
        .arg("-I")
        .arg(prefix.join("include"))
        .arg("-o")
        .arg(out.join("bridge.o"));
    if wasi {
        command.arg("--target=wasm32-wasip2");
    }
    assert!(
        command.status().unwrap().success(),
        "C++ bridge compilation failed"
    );
    assert!(
        Command::new(archiver)
            .arg("crs")
            .arg(out.join("libocr_bridge.a"))
            .arg(out.join("bridge.o"))
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    for lib in ["ocr_bridge", "tesseract", "leptonica", "z"] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    if wasi {
        println!(
            "cargo:rustc-link-search=native={}",
            sdk.as_ref()
                .unwrap()
                .join("share/wasi-sysroot/lib/wasm32-wasip2/noeh")
                .display()
        );
        println!(
            "cargo:rustc-link-search=native={}",
            sdk.unwrap()
                .join("share/wasi-sysroot/lib/wasm32-wasip2")
                .display()
        );
        for lib in [
            "c++",
            "c++abi",
            "wasi-emulated-mman",
            "wasi-emulated-signal",
            "wasi-emulated-process-clocks",
        ] {
            println!("cargo:rustc-link-lib=static={lib}");
        }
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }
    println!("cargo:rerun-if-changed=bridge.cpp");
    for name in ["libtesseract.a", "libleptonica.a", "libz.a"] {
        println!(
            "cargo:rerun-if-changed={}",
            prefix.join("lib").join(name).display()
        );
    }
    println!("cargo:rerun-if-env-changed=WASI_SDK_PATH");
}
