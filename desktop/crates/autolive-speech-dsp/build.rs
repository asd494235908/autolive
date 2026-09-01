use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor = manifest_dir.join("vendor").join("speexdsp");
    let include_dir = vendor.join("include");
    let source_dir = vendor.join("libspeexdsp");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let config = out_dir.join("config.h");

    // SpeexDSP 1.2.1 的浮点、KISS FFT 配置；不依赖系统 pkg-config 或运行时下载。
    fs::write(
        &config,
        "#ifndef AUTOLIVE_SPEEXDSP_CONFIG_H\n#define AUTOLIVE_SPEEXDSP_CONFIG_H\n#define FLOATING_POINT 1\n#define USE_KISS_FFT 1\n#define HAVE_STDINT_H 1\n#define HAVE_STDLIB_H 1\n#define HAVE_STRING_H 1\n#define HAVE_STDIO_H 1\n#define SIZEOF_INT 4\n#define SIZEOF_SHORT 2\n#define SIZEOF_LONG 8\n#define EXPORT\n#endif\n",
    )
    .expect("写入 SpeexDSP config.h 失败");

    let sources = [
        "preprocess.c",
        "jitter.c",
        "mdf.c",
        "fftwrap.c",
        "filterbank.c",
        "resample.c",
        "buffer.c",
        "scal.c",
        "kiss_fft.c",
        "kiss_fftr.c",
    ];
    let mut build = cc::Build::new();
    build
        .include(&include_dir)
        .include(&source_dir)
        .include(&out_dir)
        .define("HAVE_CONFIG_H", None)
        .define("FLOATING_POINT", None)
        .define("USE_KISS_FFT", None)
        .define("DISABLE_WARNINGS", None)
        .warnings(false)
        .flag_if_supported("-std=c99");
    for source in sources {
        let path = source_dir.join(source);
        println!("cargo:rerun-if-changed={}", path.display());
        build.file(path);
    }
    println!("cargo:rerun-if-changed={}", include_dir.display());
    build.compile("autolive_speexdsp");
}
