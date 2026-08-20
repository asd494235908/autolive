fn main() {
    println!("cargo:rustc-check-cfg=cfg(autolive_has_portaudio)");
    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor = manifest_dir
        .join("..")
        .join("..")
        .join("vendor")
        .join("portaudio");
    let include = vendor.join("include");
    let lib = vendor.join("lib");
    let bin = vendor.join("bin");
    let import_lib = lib.join("portaudio_x64.lib");
    let dll = bin.join("portaudio_x64.dll");
    let header = include.join("portaudio.h");

    println!(
        "cargo:rerun-if-changed={}",
        vendor.join("VENDOR.md").display()
    );
    println!("cargo:rerun-if-changed={}", import_lib.display());
    println!("cargo:rerun-if-changed={}", dll.display());
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:rerun-if-env-changed=AUTOLIVE_PORTAUDIO_VENDOR");

    let has_vendor = import_lib.is_file() && dll.is_file() && header.is_file();
    if cfg!(windows) && has_vendor {
        println!("cargo:rustc-cfg=autolive_has_portaudio");
        println!("cargo:rustc-link-search=native={}", lib.display());
        // ponytail: 用 DLL import lib，避开静态库 /MT 与 Rust /MD CRT 冲突
        println!("cargo:rustc-link-lib=dylib=portaudio_x64");
        println!("cargo:rustc-link-lib=dylib=ole32");
        println!("cargo:rustc-link-lib=dylib=winmm");
        println!("cargo:rustc-link-lib=dylib=user32");
        println!("cargo:rustc-link-lib=dylib=advapi32");
        println!("cargo:rustc-link-lib=dylib=setupapi");
        println!(
            "cargo:rustc-env=AUTOLIVE_PORTAUDIO_DLL_DIR={}",
            bin.display()
        );
        // 进程启动就要加载 DLL：拷到 target/{profile} 与 deps
        if let Ok(out_dir) = std::env::var("OUT_DIR") {
            let out = std::path::PathBuf::from(out_dir);
            // .../target/<profile>/build/<crate>/out → target/<profile>
            if let Some(profile_dir) = out.ancestors().nth(3) {
                let _ = std::fs::copy(&dll, profile_dir.join("portaudio_x64.dll"));
                let _ = std::fs::copy(&dll, profile_dir.join("deps").join("portaudio_x64.dll"));
            }
        }
    } else {
        println!(
            "cargo:warning=PortAudio vendor missing at {}; probe will report webview fallback",
            vendor.display()
        );
    }
}
