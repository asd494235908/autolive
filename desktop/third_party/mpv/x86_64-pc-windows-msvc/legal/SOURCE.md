# mpv Windows 运行时来源

本目录对应的 `mpv.exe` 来自下列固定发布资产：

- 构建仓库：<https://github.com/shinchiro/mpv-winbuild-cmake>
- 发布标签：`20260814`
- 资产：`mpv-x86_64-20260814-git-7b8915bc1d.7z`
- 下载地址：<https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20260814/mpv-x86_64-20260814-git-7b8915bc1d.7z>
- 资产 SHA-256：`1bf3b029da2c98e605e00e85f21ee3142f22a1dcc4ceb5c827b5c51e36e390f9`
- 构建仓库提交：`cd1edc11dc6887a50f705717619d879f5a93a488`
- 构建仓库源码归档 SHA-256：`31c619b163891b7e49fa6118189d99ec228a2ac1e32401dd4d29546ec725dfab`

## 上游源码

### mpv

- 仓库：<https://github.com/mpv-player/mpv>
- 完整提交：`7b8915bc1d04c7e1b61184e00c7fbfaab1911e75`
- 源码归档 SHA-256：`5665448fdd02a5b6a9de40b4c0a32bf546b7ab5b7e065207b66a68460666dbba`
- 许可证：`GPL-2.0-or-later`
- 随包正文：`GPL-2.0.txt`

### libplacebo

- 仓库：<https://github.com/haasn/libplacebo>
- 完整提交：`22ee762e8e0890fc54068beb670310f0edce7263`
- 源码归档 SHA-256：`eeb9adc48c1d580bfd5ba7b4a64380fa9b62c0150f1a828e8b2bc17662d40c29`
- 许可证：`LGPL-2.1-or-later`
- 随包正文：`LGPL-2.1.txt`

### FFmpeg

- 仓库：<https://github.com/FFmpeg/FFmpeg>
- `mpv --version` 报告：`N-126125-g1d7b14f61`
- 可解析完整提交：`1d7b14f61d66fdf18f15204c613df9d65396c319`
- 提交页面：<https://github.com/FFmpeg/FFmpeg/commit/1d7b14f61d66fdf18f15204c613df9d65396c319>

该引用只确定 FFmpeg 自身源码。固定构建脚本同时启用 `--enable-gpl`、`--enable-version3` 和多项外部静态库；在这些外部库的精确源码与许可证组合完成复核前，不能据此单独确定最终 `mpv.exe` 的完整许可证告知。

## 固定构建图审计

`mpv-winbuild-cmake` 的 `packages/mpv.cmake` 为该 Git 版资产声明了以下直接构建依赖：

`angle-headers`、`ffmpeg`、`fribidi`、`lcms2`、`libarchive`、`libass`、`libdvdnav`、`libdvdread`、`libiconv`、`libjpeg`、`libpng`、`luajit`、`rubberband`、`uchardet`、`openal-soft`、`mujs`、`vulkan`、`shaderc`、`libplacebo`、`spirv-cross`、`vapoursynth`、`libsdl2`、`subrandr`、`libsixel`、`curl`。

这份列表只是构建图，不是已经完成的随包第三方告知：

- 除 `libiconv 1.18` 的固定归档外，多数项目从 Git 默认分支或可移动分支获取；构建脚本没有记录本次资产所用的完整提交。
- FFmpeg 自身又声明大量编解码、字幕、字体、网络和图像库依赖，当前资产没有附带最终链接清单、SBOM 或对应源码包，不能由顶层 `DEPENDS` 反推出实际静态链接集合。
- `mpv-packaging` 从默认分支获取且没有固定提交；libplacebo 版本输出还带有 `dirty`，两者都使现有供应资产无法仅凭发布标签完整复现。
- 因此当前资产只能用于开发和技术门禁。要进入正式发布，必须由供应方提供与该二进制对应的完整构建日志、精确依赖提交、补丁、配置、SBOM 和对应源码包，或者改用本地/CI 完全固定输入的可复现构建。

## Windows D3DCompiler 策略

本产品支持 Windows 10/11 x64。固定 mpv 源码在 Windows 8.1 及以上优先从 System32 加载系统 `d3dcompiler_47.dll`。因此发布包不携带构建资产中名为 `d3dcompiler_43.dll`、但版本信息显示其内部及原始文件名为 `d3dcompiler_47.dll` 的冗余回退文件，也不声明与该文件不相符的 DirectX June 2010 D3DCompiler 43 许可映射。

## 尚未完成的发布义务

该 mpv 二进制静态包含 FFmpeg 及其他构建依赖。上游版本输出把 libplacebo 构建标记为 `dirty`，构建脚本还引用未固定提交的 `mpv-packaging` 内容；因此，仅凭上述 mpv、libplacebo 和构建仓库引用，不能证明二进制的全部对应源码和第三方版权告知已经完整。

正式发布必须另外提供并通过哈希门禁校验的 `THIRD-PARTY-NOTICES.md`，列出实际静态依赖、版本、许可证、版权和对应源码获取方式。在该文件完成法律复核前，资源准备脚本与 release/custom 构建必须保持失败，不得将本说明解释为已完成法律合规审查。
