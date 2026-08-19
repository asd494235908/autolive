# Vendored PortAudio v19.7.0 (Windows x64)

Built from https://files.portaudio.com/archives/pa_stable_v190700_20210406.tgz
Host APIs: WASAPI, WMME, WDMKS, DirectSound (ASIO off — needs Steinberg SDK).
CRT: `/MD` shared DLL (matches Rust MSVC).

- include/: headers
- lib/portaudio_x64.lib: import library
- bin/portaudio_x64.dll: runtime（由 crate build.rs 复制到开发构建输出）
- `desktop/src-tauri/portaudio/portaudio_x64.dll`: Tauri 发布包复用的同一份运行时 DLL

当前构建直接复用这些已编译产物，不在桌面构建期间下载或重新编译 PortAudio。
