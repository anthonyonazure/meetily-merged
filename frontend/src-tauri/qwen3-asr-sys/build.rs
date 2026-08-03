// build.rs for qwen3-asr-sys
//
// With the `vendor` cargo feature enabled:
//   1. Ensures vendor/qwen3-asr.cpp is present, cloning it at a PINNED commit
//      (supply-chain: never build from an unpinned upstream HEAD)
//   2. Builds GGML via cmake (produces ggml, ggml-base, ggml-cpu static libs)
//   3. Compiles vendor source files + our C wrapper via cc crate
//   4. Links everything together
//
// Without the feature: compiles only the stub C wrapper. The stub reports the
// engine as unavailable at runtime (qwen3_asr_is_available() == false), so the
// UI hides the Qwen option and no network access or vendor code is required.

/// Pinned commit of https://github.com/predict-woo/qwen3-asr.cpp
/// Resolved from upstream HEAD on 2026-08-03. Update deliberately, never
/// implicitly: review the upstream diff before bumping.
const VENDOR_REPO_URL: &str = "https://github.com/predict-woo/qwen3-asr.cpp";
const VENDOR_PINNED_COMMIT: &str = "6dcc586e5073fd6e85ee5728e75f0903d6c70c6c";

fn main() {
    println!("cargo:rerun-if-changed=qwen3_asr_c.cpp");
    println!("cargo:rerun-if-changed=qwen3_asr_c.h");

    let vendor_requested = std::env::var("CARGO_FEATURE_VENDOR").is_ok();
    let vendor_dir = std::path::Path::new("vendor/qwen3-asr.cpp");

    if vendor_requested {
        ensure_vendor_pinned(vendor_dir);
        println!("cargo:warning=Building with qwen3-asr.cpp vendor library (pinned {})", VENDOR_PINNED_COMMIT);
        build_with_vendor(vendor_dir);
    } else {
        println!("cargo:warning=Building qwen3-asr-sys WITHOUT vendor library (stub mode)");
        println!("cargo:warning=Enable the app's `qwen-asr` cargo feature to build the real engine");
        build_stub_only();
    }

    // Link C++ standard library
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=c++");

    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-lib=stdc++");
}

/// Clone (if needed) and hard-pin the vendor checkout to VENDOR_PINNED_COMMIT.
fn ensure_vendor_pinned(vendor_dir: &std::path::Path) {
    use std::process::Command;

    if !vendor_dir.join(".git").exists() {
        if let Some(parent) = vendor_dir.parent() {
            std::fs::create_dir_all(parent).expect("failed to create vendor directory");
        }
        let status = Command::new("git")
            .args(["clone", "--no-checkout", VENDOR_REPO_URL])
            .arg(vendor_dir)
            .status()
            .expect("failed to run git clone for qwen3-asr.cpp vendor library");
        assert!(status.success(), "git clone of {} failed", VENDOR_REPO_URL);
    }

    // Fetch the pinned commit if the local clone doesn't have it yet.
    let has_commit = Command::new("git")
        .args(["cat-file", "-e", &format!("{}^{{commit}}", VENDOR_PINNED_COMMIT)])
        .current_dir(vendor_dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !has_commit {
        let status = Command::new("git")
            .args(["fetch", "origin", VENDOR_PINNED_COMMIT])
            .current_dir(vendor_dir)
            .status()
            .expect("failed to run git fetch for pinned qwen3-asr.cpp commit");
        assert!(status.success(), "git fetch of pinned commit {} failed", VENDOR_PINNED_COMMIT);
    }

    // Detached checkout of the exact pinned commit (idempotent).
    let status = Command::new("git")
        .args(["checkout", "--force", VENDOR_PINNED_COMMIT])
        .current_dir(vendor_dir)
        .status()
        .expect("failed to run git checkout for pinned qwen3-asr.cpp commit");
    assert!(status.success(), "git checkout of pinned commit {} failed", VENDOR_PINNED_COMMIT);

    assert!(
        vendor_dir.join("CMakeLists.txt").exists(),
        "vendor checkout at {} is missing CMakeLists.txt",
        vendor_dir.display()
    );
}

fn build_stub_only() {
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("qwen3_asr_c.cpp")
        .warnings(false)
        .compile("qwen3_asr_c");
}

fn build_with_vendor(vendor_dir: &std::path::Path) {
    let ggml_dir = vendor_dir.join("ggml");

    // --- Step 1: Build GGML via cmake ---
    let mut ggml_cmake = cmake::Config::new(&ggml_dir);
    ggml_cmake
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("GGML_STATIC", "ON")
        .define("GGML_CPU", "ON")
        .define("GGML_OPENMP", "OFF")
        .define("GGML_METAL", "OFF")
        .define("GGML_CUDA", "OFF")
        .define("GGML_VULKAN", "OFF")
        .define("GGML_BLAS", "OFF")
        .define("GGML_BUILD_EXAMPLES", "OFF")
        .define("GGML_BUILD_TESTS", "OFF")
        // Always build GGML in Release mode to avoid _GLIBCXX_ASSERTIONS link issues
        .profile("Release");

    // macOS Metal support (future)
    #[cfg(target_os = "macos")]
    if cfg!(feature = "metal") {
        ggml_cmake.define("GGML_METAL", "ON");
    }

    let ggml_dst = ggml_cmake.build();

    // GGML installs libs to lib/ under the cmake output dir
    println!(
        "cargo:rustc-link-search=native={}/lib",
        ggml_dst.display()
    );
    // Also check build/src for in-tree builds
    println!(
        "cargo:rustc-link-search=native={}/build/src",
        ggml_dst.display()
    );
    // And the cpu backend subfolder
    println!(
        "cargo:rustc-link-search=native={}/build/src/ggml-cpu",
        ggml_dst.display()
    );

    // Link GGML static libraries (order matters for static linking)
    println!("cargo:rustc-link-lib=static=ggml");
    println!("cargo:rustc-link-lib=static=ggml-base");
    println!("cargo:rustc-link-lib=static=ggml-cpu");

    // --- Step 2: Compile vendor sources + wrapper via cc ---
    let vendor_src = vendor_dir.join("src");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .warnings(false)
        .define("QWEN3_ASR_HAS_VENDOR", None)
        // Include paths
        .include(&vendor_src)
        .include(ggml_dir.join("include"))
        .include(ggml_dst.join("include"))
        // Vendor source files
        .file(vendor_src.join("mel_spectrogram.cpp"))
        .file(vendor_src.join("gguf_loader.cpp"))
        .file(vendor_src.join("audio_encoder.cpp"))
        .file(vendor_src.join("text_decoder.cpp"))
        .file(vendor_src.join("audio_injection.cpp"))
        .file(vendor_src.join("qwen3_asr.cpp"))
        // Our C wrapper
        .file("qwen3_asr_c.cpp");

    // Optimization for release builds
    let profile = std::env::var("PROFILE").unwrap_or_default();
    if profile == "release" {
        build.opt_level(3);
    }

    build.compile("qwen3_asr_c");

    // --- Step 3: Platform-specific framework linking ---
    #[cfg(target_os = "macos")]
    {
        // Accelerate is needed for mel spectrogram (vDSP FFT)
        println!("cargo:rustc-link-lib=framework=Accelerate");

        if cfg!(feature = "metal") {
            println!("cargo:rustc-link-lib=framework=Metal");
            println!("cargo:rustc-link-lib=framework=MetalPerformanceShaders");
            println!("cargo:rustc-link-lib=framework=Foundation");
        }
    }

    #[cfg(target_os = "linux")]
    {
        if cfg!(feature = "cuda") {
            println!("cargo:rustc-link-lib=cuda");
            println!("cargo:rustc-link-lib=cublas");
        }
    }

    #[cfg(target_os = "windows")]
    {
        if cfg!(feature = "cuda") {
            println!("cargo:rustc-link-lib=cuda");
            println!("cargo:rustc-link-lib=cublas");
        }
    }
}
