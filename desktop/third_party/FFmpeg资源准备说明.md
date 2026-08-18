# FFmpeg 资源准备说明

桌面端正式产物会把 FFmpeg 和 FFprobe 放入 Tauri 资源目录。Windows x64 交付解压即用的 portable ZIP，不生成 MSI/NSIS；用户不需要单独安装 FFmpeg。

## 目标目录

将已审核并记录来源的 FFmpeg 构建放入以下目录：

```text
desktop/third_party/ffmpeg/
  x86_64-apple-darwin/ffmpeg
  x86_64-apple-darwin/ffprobe
  aarch64-apple-darwin/ffmpeg
  aarch64-apple-darwin/ffprobe
  x86_64-pc-windows-msvc/ffmpeg.exe
  x86_64-pc-windows-msvc/ffprobe.exe
```

执行 `cd desktop/ui && pnpm run tauri:build` 时，准备脚本只选择当前构建目标，将文件复制到 `desktop/src-tauri/binaries/ffmpeg[.exe]` 和 `ffprobe[.exe]`。Windows 通过正式 Tauri `--no-bundle` 构建生成 EXE，再把清单和完整 `embedded-runtime-resources/` 与 EXE 相邻归档为 portable ZIP。

可使用以下环境变量进行 CI 或本地构建：

- `AUTOLIVE_FFMPEG_SOURCE_DIR`：替换第三方资源根目录。
- `AUTOLIVE_FFMPEG_OUTPUT_DIR`：替换 Tauri 临时资源输出目录。
- `AUTOLIVE_TARGET_TRIPLE`：交叉构建时显式指定目标三元组。

## 发布前检查

- 二进制必须与目标三元组匹配，并能执行 `ffmpeg -version`、`ffprobe -version`。
- 记录 FFmpeg 版本、构建参数、来源、SHA-256 和对应源码归档。
- 许可证不能仅根据文件名推断，必须以实际构建参数和随包许可证材料为准。若要求 LGPL-only，构建参数不得启用 `--enable-gpl` 或 `--enable-nonfree`；本次 macOS 资源实测包含 `--enable-gpl`，因此当前不能标记为 LGPL-only，正式发布前必须完成许可证复核或替换为 LGPL-only 构建。
- 发布产物附带 FFmpeg 的许可证、版权声明和源码获取说明；许可证问题不由准备脚本自动判断。
- macOS 安装包还需要对内置可执行文件进行签名并通过 notarization 验证。

## 本次已下载资源

下载时间：2026-08-14。FFmpeg 官网说明其主要发布源码，预编译包由下载页列出的构建方提供；本次来源均可从 FFmpeg 下载页追溯。

### macOS Intel / Apple Silicon

来源：[Martin Riedl FFmpeg Build Server](https://ffmpeg.martin-riedl.de/)，Release 9.0；下载页标注 macOS 二进制提供签名/公证信息，但本项目仍需在最终安装包阶段重新完成应用签名与 notarization。

- [Intel ffmpeg.zip](https://ffmpeg.martin-riedl.de/download/macos/amd64/1785871427_9.0/ffmpeg.zip)：`79d14663d8b078dbbc38de18d63a30f8a5bfc860af5dfee7f8cf3e387cf1c02c`
- [Intel ffprobe.zip](https://ffmpeg.martin-riedl.de/download/macos/amd64/1785871427_9.0/ffprobe.zip)：`a2dd3f2e7eb35a10fa6ac43b1a8c21890f27bee0dc4a86ddee16a57d72d3898d`
- [Apple Silicon ffmpeg.zip](https://ffmpeg.martin-riedl.de/download/macos/arm64/1785863997_9.0/ffmpeg.zip)：`5267ef149ee0d208057a1b316aac079b661b0476574dee5da7d225769773c603`
- [Apple Silicon ffprobe.zip](https://ffmpeg.martin-riedl.de/download/macos/arm64/1785863997_9.0/ffprobe.zip)：`7778fbb533fb60d3336cbd9a9e51eced71658f020b570c7203590c1c41d42f50`

Apple Silicon 本机执行验证为 FFmpeg `9.0-https://www.martin-riedl.de`；构建配置包含 `--enable-gpl`，不包含 `--enable-nonfree`。Intel 资源已通过 `file` 校验为 Mach-O `x86_64`，Apple Silicon 资源已通过 `file` 校验为 Mach-O `arm64`。

### Windows x64

来源：[FFmpeg 官网下载页](https://ffmpeg.org/download.html)列出的 [BtbN FFmpeg Builds](https://github.com/BtbN/FFmpeg-Builds/releases)；使用 `ffmpeg-n8.1-latest-win64-lgpl-8.1.zip`。

- [Windows x64 LGPL 压缩包](https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-win64-lgpl-8.1.zip)：`b48d1f513a728a0e5ad8f51d91a0f508fe50f0a4f8de3bd3874cc5628cca5140`（上游 `latest` 于 2026-08-14 发布的资产摘要）

Windows `ffmpeg.exe` 和 `ffprobe.exe` 已通过 `file` 校验为 PE32+ x86-64。当前 macOS 环境不执行 Windows PE 文件，Windows 原生安装包验收仍需在 Windows x64 环境执行 `-version` 和媒体样片测试。

FFmpeg 的许可证和分发要求以[官方说明](https://www.ffmpeg.org/legal.html)及实际构建配置为准。
