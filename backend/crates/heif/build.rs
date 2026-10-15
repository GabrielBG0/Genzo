//! Builds libde265 and libheif from the pinned submodules in `vendor/` and
//! links them statically, so every platform ships the same decoder.
//!
//! Only the libde265 HEVC decoder is enabled. Every other codec, plugin
//! loading and optional dependency is switched off, and CMake is told not to
//! look for them, so whatever happens to be installed on the build machine
//! (x265, aom, ffmpeg, ...) can never end up in the binary.

use std::path::{Path, PathBuf};

/// Codecs and optional libraries libheif can pick up. All disabled.
const OTHER_CODECS: &[&str] = &[
    "FFMPEG_DECODER",
    "WEBCODECS",
    "X265",
    "KVAZAAR",
    "AOM_DECODER",
    "AOM_ENCODER",
    "DAV1D",
    "SvtEnc",
    "RAV1E",
    "JPEG_DECODER",
    "JPEG_ENCODER",
    "X264",
    "OpenH264_DECODER",
    "OpenJPEG_DECODER",
    "OpenJPEG_ENCODER",
    "OPENJPH_ENCODER",
    "UVG266",
    "VVENC",
    "VVDEC",
];

/// CMake packages libheif's build may search for. None may be found.
const NO_FIND: &[&str] = &[
    "X265",
    "kvazaar",
    "UVG266",
    "vvdec",
    "vvenc",
    "X264",
    "OpenH264",
    "DAV1D",
    "AOM",
    "SvtEnc",
    "RAV1E",
    "JPEG",
    "OpenJPEG",
    "FFMPEG",
    "OPENJPH",
    "libsharpyuv",
    "ZLIB",
    "Brotli",
    "Doxygen",
    "SDL",
    "SDL2",
    "PNG",
    "TIFF",
    "GdkPixbuf",
];

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor = manifest.join("vendor");
    let de265_src = vendor.join("libde265");
    let heif_src = vendor.join("libheif");
    for dir in [&de265_src, &heif_src] {
        if !dir.join("CMakeLists.txt").exists() {
            panic!(
                "{} is empty. Run `git submodule update --init --recursive` in the repository root.",
                dir.display()
            );
        }
        println!(
            "cargo:rerun-if-changed={}",
            dir.join("CMakeLists.txt").display()
        );
    }
    println!("cargo:rerun-if-changed=shim/shim.c");
    println!("cargo:rerun-if-changed=build.rs");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap();

    // libde265: the decoder library only, no tools.
    let de265 = cmake::Config::new(&de265_src)
        // Release everywhere: a debug CMake build on MSVC links the debug C
        // runtime, which conflicts with Rust's, and decoding should stay fast
        // in debug builds of the app.
        .profile("Release")
        .out_dir(out.join("libde265"))
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .define("CMAKE_INSTALL_LIBDIR", "lib")
        .define("ENABLE_SDL", "OFF")
        .define("ENABLE_DECODER", "OFF")
        .define("ENABLE_ENCODER", "OFF")
        .define("ENABLE_SHERLOCK265", "OFF")
        .define("ENABLE_INTERNAL_DEVELOPMENT_TOOLS", "OFF")
        .define("WITH_FUZZERS", "OFF")
        .define("CMAKE_DISABLE_FIND_PACKAGE_SDL", "ON")
        .define("CMAKE_DISABLE_FIND_PACKAGE_SDL2", "ON")
        .build();
    let de265_lib = find_lib(&de265.join("lib"), &["de265", "libde265"]);

    // libheif: built-in libde265 decoder, nothing else.
    let mut heif = cmake::Config::new(&heif_src);
    heif.profile("Release")
        .out_dir(out.join("libheif"))
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .define("CMAKE_INSTALL_LIBDIR", "lib")
        .define("ENABLE_PLUGIN_LOADING", "OFF")
        .define("WITH_LIBDE265", "ON")
        .define("WITH_LIBDE265_PLUGIN", "OFF")
        .define("LIBDE265_INCLUDE_DIR", de265.join("include"))
        .define("LIBDE265_LIBRARY", &de265_lib)
        .define("WITH_LIBSHARPYUV", "OFF")
        .define("WITH_UNCOMPRESSED_CODEC", "OFF")
        .define("WITH_HEADER_COMPRESSION", "OFF")
        .define("WITH_EXAMPLES", "OFF")
        .define("WITH_EXAMPLE_HEIF_THUMB", "OFF")
        .define("WITH_EXAMPLE_HEIF_VIEW", "OFF")
        .define("WITH_GDK_PIXBUF", "OFF")
        .define("BUILD_TESTING", "OFF")
        .define("BUILD_DOCUMENTATION", "OFF")
        .define("BUILD_DEVELOPMENT_TOOLS", "OFF")
        // libheif does not define this itself when linking a static libde265.
        // Without it, Windows builds reference DLL-import symbols.
        .cflag("-DLIBDE265_STATIC_BUILD")
        .cxxflag("-DLIBDE265_STATIC_BUILD");
    for codec in OTHER_CODECS {
        heif.define(format!("WITH_{codec}"), "OFF");
        heif.define(format!("WITH_{codec}_PLUGIN"), "OFF");
    }
    for pkg in NO_FIND {
        heif.define(format!("CMAKE_DISABLE_FIND_PACKAGE_{pkg}"), "ON");
    }
    let heif = heif.build();
    let heif_lib = find_lib(&heif.join("lib"), &["heif", "libheif"]);

    // The shim is compiled against the installed headers. Its link directive
    // comes first, so the static libraries it needs follow it.
    cc::Build::new()
        .file("shim/shim.c")
        .include(heif.join("include"))
        .define("LIBHEIF_STATIC_BUILD", None)
        .warnings(true)
        .compile("genzo_heif_shim");

    println!(
        "cargo:rustc-link-search=native={}",
        heif_lib.parent().unwrap().display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        de265_lib.parent().unwrap().display()
    );
    println!("cargo:rustc-link-lib=static={}", lib_name(&heif_lib));
    println!("cargo:rustc-link-lib=static={}", lib_name(&de265_lib));
    match (target_os.as_str(), target_env.as_str()) {
        ("macos" | "ios", _) => println!("cargo:rustc-link-lib=c++"),
        ("windows", "msvc") => {}
        _ => println!("cargo:rustc-link-lib=stdc++"),
    }
    // libde265's x86 code detects SSE and AVX with __builtin_cpu_supports,
    // which needs compiler-rt (___cpu_model, ___cpu_indicator_init). rustc
    // links with -nodefaultlibs, so on Intel Macs clang's runtime has to be
    // named; elsewhere libgcc or the MSVC runtime already provide it.
    if target_os == "macos" && std::env::var("CARGO_CFG_TARGET_ARCH").unwrap() == "x86_64" {
        let compiler = cc::Build::new().get_compiler();
        let resource_dir = std::process::Command::new(compiler.path())
            .arg("-print-resource-dir")
            .output()
            .expect("run the C compiler for its resource directory");
        let darwin =
            PathBuf::from(String::from_utf8_lossy(&resource_dir.stdout).trim()).join("lib/darwin");
        assert!(
            darwin.join("libclang_rt.osx.a").exists(),
            "libclang_rt.osx.a not found in {}",
            darwin.display()
        );
        println!("cargo:rustc-link-search=native={}", darwin.display());
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
    }
    // Lets dependents find the headers if they ever need them.
    println!("cargo:include={}", heif.join("include").display());
}

/// Finds a static library by its base name in `dir`, whatever the platform's
/// naming (`libheif.a`, `heif.lib`, `libheif.lib`).
fn find_lib(dir: &Path, names: &[&str]) -> PathBuf {
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    for path in &entries {
        let file = path.file_name().unwrap().to_string_lossy();
        for name in names {
            for candidate in [
                format!("lib{name}.a"),
                format!("{name}.lib"),
                format!("{name}.a"),
            ] {
                if file == candidate.as_str() {
                    return path.clone();
                }
            }
        }
    }
    panic!(
        "no static library {names:?} in {}; found {entries:?}",
        dir.display()
    );
}

/// The name rustc expects in `-l static=<name>`.
fn lib_name(path: &Path) -> String {
    let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
    let is_msvc_lib = path.extension().is_some_and(|e| e == "lib");
    match stem.strip_prefix("lib") {
        Some(rest) if !is_msvc_lib => rest.to_owned(),
        _ => stem,
    }
}
