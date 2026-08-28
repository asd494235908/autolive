# mpv 实时 GPU 资源准备说明

Windows x64 画面主链使用 mpv 的 `gpu-next`/libplacebo，并由桌面进程以固定参数嵌入最终效果窗口。mpv 只负责实时画面；现有 FFmpeg/FFprobe 继续承担媒体探测、普通声音处理和必要的一次性视频导入兼容，不恢复逐周期视频文件回退。

## 固定版本

- 上游构建仓库：`shinchiro/mpv-winbuild-cmake`
- Release：`20260814`
- 资产：`mpv-x86_64-20260814-git-7b8915bc1d.7z`
- 下载地址：`https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20260814/mpv-x86_64-20260814-git-7b8915bc1d.7z`
- 资产 SHA-256：`1bf3b029da2c98e605e00e85f21ee3142f22a1dcc4ceb5c827b5c51e36e390f9`
- mpv 实测报告：`v0.41.0-923-g7b8915bc1`，构建时间 `Aug 14 2026 00:27:31`；资产名固定构建引用为 `7b8915bc1d`
- libplacebo 实测报告：`v7.371.0 (v7.360.0-111-g22ee762-dirty)`，构建提交标识 `22ee762`

只提取 `mpv.exe` 与 `d3dcompiler_43.dll` 到：

```text
desktop/third_party/mpv/x86_64-pc-windows-msvc/
```

同一目标目录必须提供固定发布材料：

```text
legal/
  mpv-runtime-manifest.json
  Copyright.txt
  GPL-2.0.txt
  LGPL-2.1.txt
  SOURCE.md
  D3DCOMPILER_43-EULA.txt
```

`mpv-runtime-manifest.json` 必须记录并通过构建脚本精确校验：上述构建仓库、Release、资产、下载地址和资产 SHA-256；mpv 的实测版本、构建时间、源码仓库及资产中的完整提交引用；libplacebo 的实测版本、`dirty` 构建修订、源码仓库及提交标识；mpv 的 `GPL-2.0-or-later`、libplacebo 的 `LGPL-2.1-or-later`、D3DCompiler 的 Microsoft 再分发许可映射；以及提取后 `mpv.exe`、`d3dcompiler_43.dll` 和 `legal/Copyright.txt`、`legal/GPL-2.0.txt`、`legal/LGPL-2.1.txt`、`legal/SOURCE.md`、`legal/D3DCOMPILER_43-EULA.txt` 的实际 SHA-256。`SOURCE.md` 记录对应源码和构建脚本的获取方式，且必须说明 libplacebo 的 `dirty` 标记意味着不能只凭标签声称可复现；`Copyright.txt` 汇总实际分发组件的版权声明。许可证结论来自清单中的组件映射和随包正文，不根据文件名、非空占位内容或 `mpv --version` 推断。

构建准备脚本会把二进制与 FFmpeg/FFprobe 一起复制到 Tauri 的受管 `binaries` 目录，并把全部发布材料复制到明确的 `binaries/licenses/mpv/`；后续运行资源发布树会递归纳入该目录及 SHA-256 清单。任一材料缺失、为空、固定字段不一致，或二进制/法律材料哈希不匹配，都会在创建输出前失败。干净 CI 必须显式供应同一固定资产及经法律复核后提交的发布材料；当前工作流尚未供应这些材料，因此 Windows 正式包保持阻断。不得改为自动下载 latest，也不得引入归档中的安装器、更新器、注册脚本、文档或用户配置。

## 升级、回滚与删除

- 升级时先在独立临时目录下载明确 Release 资产，校验归档 SHA-256，只提取白名单二进制；随后重新实测 `--version`、D3D11/Vulkan/CPU4 门禁，更新源码引用、全部文件哈希和法律材料，经复核后再原子替换固定资源目录。不得在原目录上增量覆盖。
- 新版本任一功能、性能、哈希或法律门禁失败时，回滚为上一份已审核的完整资源目录及对应清单；生产会话同时保持 Original，不恢复旧 FFmpeg/MSE 视频周期链。
- 删除 mpv 方案时，先移除 Tauri bundle/运行资源清单和生产解析入口，再删除 `binaries/mpv.exe`、`binaries/d3dcompiler_43.dll`、`binaries/licenses/mpv/` 及 third-party 固定目录；最后执行资源树生成和干净安装/卸载验证，确认没有陈旧 DLL、许可证文件或运行进程。删除不影响 FFmpeg/FFprobe 与声音链。

## 实测能力

本固定构建公开 `d3d11`、`vulkan`、`opengl` GPU API；Windows 上的上下文包括 `d3d11` 与 `winvk`。桌面主链依次实际探测 D3D11/D3D11VA 和 Vulkan，不以帮助列表代替真实媒体启动。

## 许可证与当前阻断

上游 mpv 默认声明为 `GPL-2.0-or-later`，libplacebo 声明为 `LGPL-2.1-or-later`；`d3dcompiler_43.dll` 必须以其真实 Microsoft 再分发条款为准。当前 mpv 可执行文件还静态带入 FFmpeg 和多项构建依赖，不能仅凭这两条组件许可证推断整个二进制的完整义务，也不得把整个二进制标记为 LGPL-only。发布包必须附带固定清单、实际依赖版权清单、GPL/LGPL 正文、D3DCompiler 许可、构建来源以及对应源码获取说明，并经发布方完成法律复核。

当前仓库仅有二进制和哈希清单，缺少 `Copyright.txt`、GPL/LGPL 正文、`SOURCE.md` 与 D3DCompiler EULA；`prepare-ffmpeg-resources.mjs` 因此会在复制前非零退出。这是预期的 fail-closed 状态，不能写成许可证或发布门禁已经通过。

## 体积门禁

原始 `mpv.exe` 约 114 MiB，发布归档约 32 MiB；NSIS 压缩后预期使当前约 279 MB 安装包增加约 25–40 MB。最终安装包超过 320 MB 时必须先列出压缩后资源明细和移除方案。
