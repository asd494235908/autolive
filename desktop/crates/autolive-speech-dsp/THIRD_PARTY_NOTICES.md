# 第三方组件

本 crate 内的 `vendor/speexdsp` 来自 SpeexDSP 1.2.1 官方发布归档：

- 来源：<https://ftp.osuosl.org/pub/xiph/releases/speex/speexdsp-1.2.1.tar.gz>
- SHA-256：`8C777343E4A6399569C72ABC38A95B24DB56882C83DBDB6C6424A5F4AEB54D3D`
- 上游镜像：<https://github.com/xiph/speexdsp>
- 许可证：BSD-3-Clause，完整文本见 `vendor/speexdsp/COPYING`。
- Windows 安装包同时携带 `src-tauri/speexdsp/LICENSE.txt`，用于满足二进制再分发的许可证告知要求。

只编译 SpeexDSP 的预处理、回声消除、KISS FFT 及其必要实现文件；不使用
系统 `pkg-config`、运行时下载或未锁定的动态库。构建在本地开发机或 CI 完成，
不在服务器构建源码。
