fn main() {
    println!("cargo:rerun-if-changed=src/bridge.cpp");
    println!("cargo:rerun-if-changed=src/bridge.h");
    println!("cargo:rerun-if-changed=vendor/signalsmith-stretch/signalsmith-stretch.h");
    println!("cargo:rerun-if-changed=vendor/signalsmith-linear/stft.h");
    println!("cargo:rerun-if-changed=vendor/signalsmith-linear/fft.h");

    cc::Build::new()
        .cpp(true)
        .std("c++14")
        // Signalsmith 官方说明 Debug 未优化可慢约 10 倍；只优化该数值计算静态库。
        .opt_level(2)
        .include("vendor")
        .file("src/bridge.cpp")
        .compile("autolive_signalsmith_stretch");
}
