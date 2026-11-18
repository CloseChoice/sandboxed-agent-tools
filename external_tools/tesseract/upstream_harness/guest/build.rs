use std::{env, path::PathBuf, process::Command};
fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let sdk = PathBuf::from(env::var_os("WASI_SDK_PATH").expect("WASI_SDK_PATH"));
    let upstream = PathBuf::from(env::var_os("TESSERACT_UPSTREAM").expect("TESSERACT_UPSTREAM"));
    let prefix = PathBuf::from(env::var_os("OCR_PREFIX").expect("OCR_PREFIX"));
    let gtest = PathBuf::from(env::var_os("GTEST_SOURCE").expect("GTEST_SOURCE"));
    let gtest_build = PathBuf::from(env::var_os("GTEST_BUILD").expect("GTEST_BUILD"));
    let name = env::var("UPSTREAM_TEST").expect("UPSTREAM_TEST");
    assert!(name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut objects = Vec::new();
    for (source, object) in [
        (root.join("main.cpp"), "main.o"),
        (
            upstream.join("src/training/unicharset/fileio.cpp"),
            "fileio.o",
        ),
        (
            upstream.join("unittest").join(format!("{name}.cc")),
            "test.o",
        ),
    ] {
        let mut command = Command::new(sdk.join("bin/clang++"));
        command.args([
            "--target=wasm32-wasip2",
            "-std=c++20",
            "-O2",
            "-fno-exceptions",
            "-DHAVE_CONFIG_H",
            "-DTESSERACT_DISABLE_DEBUG_FONTS",
            "-DTESS_UNICHARSET_TRAINING_API=",
            "-DTESS_COMMON_TRAINING_API=",
            "-DGTEST_HAS_PTHREAD=0",
            "-DGTEST_HAS_STREAM_REDIRECTION=0",
            "-DGTEST_HAS_POSIX_RE=0",
            "-DGTEST_HAS_EXCEPTIONS=0",
        ]);
        command
            .arg("-include")
            .arg(prefix.parent().unwrap().join("tesseract/config_auto.h"));
        for directory in [
            prefix.parent().unwrap().join("tesseract"),
            prefix.join("include"),
            prefix.join("include/leptonica"),
            upstream.join("include"),
            upstream.join("unittest"),
            gtest.join("googletest/include"),
            gtest.join("googlemock/include"),
        ] {
            command.arg("-I").arg(directory);
        }
        for directory in [
            "arch", "ccmain", "ccstruct", "ccutil", "classify", "cutil", "dict", "lstm", "textord",
            "viewer", "wordrec",
        ] {
            command.arg("-I").arg(upstream.join("src").join(directory));
        }
        command.arg("-I").arg(upstream.join("src/training/common"));
        command
            .arg("-I")
            .arg(upstream.join("src/training/unicharset"));
        command.arg(format!(
            "-DTESTING_DIR=\"{}\"",
            upstream.join("test/testing").display()
        ));
        command.arg(format!(
            "-DTESTDATA_DIR=\"{}\"",
            upstream.join("test/testdata").display()
        ));
        command.arg(format!(
            "-DTESSDATA_DIR=\"{}\"",
            upstream.parent().unwrap().join("tessdata").display()
        ));
        if let Some(data) = env::var_os("LANGDATA_DIR") {
            command.arg(format!(
                "-DLANGDATA_DIR=\"{}\"",
                PathBuf::from(data).display()
            ));
        }
        if object == "test.o" {
            if let Some(tmp) = env::var_os("SAT_TEST_TMPDIR") {
                command.arg(format!(
                    "-DSAT_TEST_TMPDIR=\"{}\"",
                    PathBuf::from(tmp).display()
                ));
                command.arg("-include").arg(root.join("test_environment.h"));
            }
        }
        assert!(
            command
                .arg("-c")
                .arg(source)
                .arg("-o")
                .arg(out.join(object))
                .status()
                .unwrap()
                .success(),
            "test compilation failed"
        );
        objects.push(out.join(object));
    }
    assert!(
        Command::new(sdk.join("bin/llvm-ar"))
            .arg("crs")
            .arg(out.join("liboriginal_test.a"))
            .args(objects)
            .status()
            .unwrap()
            .success()
    );
    for directory in [
        out,
        prefix.join("lib"),
        gtest_build.join("lib"),
        sdk.join("share/wasi-sysroot/lib/wasm32-wasip2/noeh"),
        sdk.join("share/wasi-sysroot/lib/wasm32-wasip2"),
    ] {
        println!("cargo:rustc-link-search=native={}", directory.display());
    }
    println!("cargo:rustc-link-lib=static:+whole-archive=original_test");
    for lib in [
        "gmock",
        "gtest",
        "tesseract",
        "leptonica",
        "z",
        "c++",
        "c++abi",
        "wasi-emulated-mman",
        "wasi-emulated-signal",
        "wasi-emulated-process-clocks",
    ] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    for key in [
        "UPSTREAM_TEST",
        "TESSERACT_UPSTREAM",
        "OCR_PREFIX",
        "GTEST_SOURCE",
        "GTEST_BUILD",
        "WASI_SDK_PATH",
        "SAT_TEST_TMPDIR",
        "LANGDATA_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    println!("cargo:rerun-if-changed=main.cpp");
    println!("cargo:rerun-if-changed=test_environment.h");
    println!(
        "cargo:rerun-if-changed={}",
        upstream
            .join("unittest")
            .join(format!("{name}.cc"))
            .display()
    );
}
