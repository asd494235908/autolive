# PortAudio (vendored runtime)

- Binary: portaudio_x64.dll (v19.7.0, Windows x64, WASAPI/WMME/WDMKS/DS; ASIO off)
- License: see LICENSE.txt (MIT-style PortAudio license)
- 构建直接复用项目内已编译 DLL，不在桌面构建期间重新编译 PortAudio。
- ASIO Host API requires Steinberg ASIO SDK and is not enabled in this build.
- The desktop UI still shows ASIO as a disabled Host API option until this runtime actually enumerates an ASIO output device.
