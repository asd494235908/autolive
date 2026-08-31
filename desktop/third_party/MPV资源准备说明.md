# mpv 实时 GPU 资源准备说明

Windows x64 画面主链使用 mpv 的 `gpu-next`/libplacebo，并由桌面进程以固定参数嵌入最终效果窗口。mpv 只负责实时画面；现有 FFmpeg/FFprobe 继续承担媒体探测、普通声音处理和必要的一次性视频导入兼容，不恢复逐周期视频文件回退。

## 固定源码与候选

- 目标：`x86_64-pc-windows-msvc`
- mpv：`7b8915bc1d04c7e1b61184e00c7fbfaab1911e75`
- FFmpeg：`1d7b14f61d66fdf18f15204c613df9d65396c319`，启用 GPL、D3D11VA 与常用视频编解码器的 D3D11VA 硬解组件，许可证身份固定为 `GPL-2.0-or-later`
- libplacebo：`22ee762e8e0890fc54068beb670310f0edce7263`
- 当前整改锁 SHA-256：`cff165702cadadb3bcb220c90466e46933a40f47796b4cbd0c08fe4d352a5352`
- 构建镜像：`ghcr.io/llvm/ci-ubuntu-24.04@sha256:224c58f5d5f3f1d4b8f36dd3873b00a5d60c28065693165d875a9454ed914233`

旧 `shinchiro/mpv-winbuild-cmake` 资产只保留为开发基线和历史门禁，不再是正式发布候选。Phase 7A 的断网构建输出三项运行二进制到独立证据目录；Phase 7B 全部通过前不得覆盖当前开发资源：

```text
build-evidence/
  mpv.exe
  spirv-cross-c-shared.dll
  vulkan-1.dll
```

同一目标目录必须提供固定发布材料：

```text
legal/
  mpv-runtime-manifest.json
  Copyright.txt
  GPL-2.0.txt
  LGPL-2.1.txt
  SOURCE.md
  THIRD-PARTY-NOTICES.md
```

`mpv-runtime-manifest.json` schema v2 必须记录并通过构建脚本精确校验：当前锁与 Phase 7A 报告哈希、构建 claim/scope、mpv/libplacebo/FFmpeg 固定源码身份、三项运行二进制和五项法律材料的实际 SHA-256，以及报告、对应源码、CycloneDX、SPDX、许可证清单和版权清单六项供应证据。技术模式允许法律审核保持阻断；release 模式只接受 `audit.release_review_status=approved`、`audit.corresponding_source_complete=true` 和 `audit.third_party_notices_reviewed=true`。只补一个非空通知文件不能解除发布阻断。许可证结论来自固定锁、实际构建参数和经复核正文，不根据文件名、非空占位内容或 `mpv --version` 推断。

构建准备脚本在 Phase 7B 技术与法律门禁全部通过后，才把三项候选二进制与 FFmpeg/FFprobe 一起复制到 Tauri 受管 `binaries`，并把内部清单和五项发布材料复制到 `binaries/licenses/mpv/`；运行资源发布树目标严格为十一项：FFmpeg、FFprobe、三项候选二进制、内部清单和五项法律文件。二进制条目必须标记 `executable: true`，`binaries/licenses/` 下材料必须标记 `executable: false`。任一必需材料缺失、审核未批准、对应源码或第三方告知未完成、字段或哈希不匹配，都会在创建输出前失败。准备树、发布树、内嵌树和外层清单只从互不相同且不相互嵌套的同级 staging 路径事务提交；任一搬移失败回滚已替换目标。内部 schema v2 清单除字段语义外还必须保持准入时的完整文件大小和 SHA-256，等价重排 JSON 也视为身份变化。Windows release/custom 构建会再次双向核对文件集、大小和 SHA-256；额外声明、未声明文件、重复、越界或嵌套路径均阻断。干净 CI 必须显式供应同一固定资产和经法律复核的发布材料，不得下载 latest。

Phase 7A 的唯一可复现构建输入契约是 [`mpv/reproducible-build-lock.json`](./mpv/reproducible-build-lock.json)。`pnpm --dir desktop/ui audit:mpv-reproducible-lock` 只做静态审计，不启动构建器、进程或网络；构建流水线必须额外使用 `node desktop/tools/verify-mpv-reproducible-build-lock.mjs --require-metadata-complete`。锁要求 Windows MSVC 目标、HTTPS 取源/断网构建分段、Git 完整提交或不可移动的归档 HTTPS URL、源码缓存路径与 SHA-256 成对、补丁顺序、摘要固定的构建镜像、Windows SDK/UCRT 在内的精确工具输入、确定性环境、`--wrap-mode=nodownload`、`auto_features=disabled`、`build-date=false`、静态链接、FFmpeg 禁网、固定视频功能集合、PE 动态依赖允许列表和完整输出角色。Meson/FFmpeg 参数只允许固定集合，任何重复、相反开关或未知项都失败关闭；`cache_inventory` 必须与源码、补丁、工具链缓存双向完全相等。输入锁的 `metadata_complete` 只表示已声明输入满足固定结构，不证明缓存文件、依赖闭包或构建结果真实完整；后续文件和语义门禁必须独立复算，三个根组件本身不构成供应候选准入证据。

物化后的源码、工具链和补丁归档统一放在 `mpv/build-inputs/`，目录相对路径必须与 `cache_inventory` 双向完全相等。每个条目在锁中同时记录 `size_bytes` 和 SHA-256，声明总量不得超过 32 GiB。Dockerfile、主脚本、cross file、配置和被调用模块等全部构建配方文件由 `recipe_inventory` 管理，固定 `mpv/build/` 下实际文件集合必须与其双向相等；空目录不作为构建输入，没有准入语义。`pnpm --dir desktop/ui audit:mpv-reproducible-inputs` 会拒绝符号链接、目录联接越界、额外/缺失文件和有界目录之外的路径，通过同一文件句柄复核大小、哈希前后身份。元数据未完成时不会读取不存在的缓存；全部字节匹配也只输出 `input_bytes_match_lock/admitted=false`，它证明自洽性而非可信来源，不代表已经构建或发布。

输入锁不保存 `status/audit/unresolved` 等自证完成字段；`blocked/metadata_complete` 完全由校验器根据声明缺口派生。SBOM、对应源码、许可证、产物哈希和 PE 导入表属于构建后证据，必须写入独立 `reproducible-build-report.json`。锁 `68ff115a...` 的首次整改构建在约 37 分钟时因宿主 Docker/WSL 引擎管道消失以 Exit `255` 中断，容器与 nocopy 卷保留，不能作为候选证据。根因整改把构建并发从宿主 `nproc` 改为固定 `2`，并把容器内存与 memory-swap 同时固定为 `4 GiB`；当前锁 `cff16570...` 已通过 `metadata_complete` 与 `input_bytes_match_lock`，26 项缓存输入均按大小和 SHA-256 复核，新的全新卷断网冷构建正在执行。旧锁 `5ee096...` 的已准入报告因锁身份不同只作历史证据，不能证明当前候选。新报告仍必须通过文件集、内容语义和精确 claim 三层门禁后才允许进入 Phase 7B。

构建锁里的 `wasapi/openal/sdl2-audio=disabled` 只裁剪这个外部、视频专用 mpv 可执行文件的音频输出能力，因为生产启动本就使用 `--audio=no`。它不修改、不替代也不测试项目的 PortAudio、普通声音、插话、固定话术、音频候选或音频参数链。

Windows 10/11 使用系统 System32 中的 `d3dcompiler_47.dll`。固定 mpv 源码在 Windows 8.1 及以上先加载该系统组件，随发布资产携带的回退文件虽然名为 `d3dcompiler_43.dll`，其版本信息却表明内部和原始文件名都是 `d3dcompiler_47.dll`，因此本产品不分发该冗余且身份误导的副本，也不附加不匹配的 DirectX June 2010 D3DCompiler 43 EULA。准备脚本在成功发布新资源树时会删除旧生成目录中的这两个陈旧输出。

## 升级、回滚与删除

- 升级时先在独立临时目录下载明确 Release 资产，校验归档 SHA-256，只提取白名单二进制；随后重新实测 `--version`、D3D11/Vulkan/CPU4 门禁，更新源码引用、全部文件哈希和法律材料，经复核后再原子替换固定资源目录。不得在原目录上增量覆盖。
- 新版本任一功能、性能、哈希或法律门禁失败时，回滚为上一份已审核的完整资源目录及对应清单；生产会话同时保持 Original，不恢复旧 FFmpeg/MSE 视频周期链。
- 删除 mpv 方案时，先移除 Tauri bundle/运行资源清单和生产解析入口，再删除 `binaries/mpv.exe`、`binaries/licenses/mpv/` 及 third-party 固定目录；最后执行资源树生成和干净安装/卸载验证，确认没有陈旧 DLL、许可证文件或运行进程。删除不影响 FFmpeg/FFprobe 与声音链。

## 实测能力

本固定构建公开 `d3d11`、`vulkan`、`opengl` GPU API；Windows 上的上下文包括 `d3d11` 与 `winvk`。桌面主链依次实际探测 D3D11/D3D11VA 和 Vulkan，不以帮助列表代替真实媒体启动。

这里的 D3D11/Vulkan 是外部 `mpv.exe` 的 VO/GPU API，不能等同于 libmpv 面向宿主公开的 Render API。当前运行资产只有 `mpv.exe`，没有 libmpv DLL、导入库、`client.h/render.h` 或 ABI 清单；固定引用 `7b8915bc1d` 的公开 `render.h` 经官方主源复核只声明 OpenGL 与软件后端，但仓库没有本地头文件，因此审计报告不会把它伪装成本地校验。`pnpm --dir desktop/ui audit:mpv-phase3c-admission` 成功时仍输出 `status/admissionStatus=not_admitted`、`checkStatus=passed`、生产 `61/83`；这表示跨帧历史仍不准入，不否定已验证的 61 项同帧能力。未来新增任一 libmpv 资产、Cargo 别名依赖、运行资源/构建声明或 FFI 前，必须重新审核版本、哈希、架构、许可证、主 crate `unsafe_code = "forbid"` 的隔离策略、D3D11/Vulkan 互操作和删除路径；不得把 DLL 悄悄加入现有清单。

## 许可证与当前阻断

上游 mpv 声明为 `GPL-2.0-or-later`，libplacebo 声明为 `LGPL-2.1-or-later`。当前 mpv 可执行文件还静态带入 FFmpeg 和多项构建依赖，不能仅凭这两条组件许可证推断整个二进制的完整义务，也不得把整个二进制标记为 LGPL-only。发布包必须附带固定清单、实际依赖版权清单、GPL/LGPL 正文、构建来源以及对应源码获取说明，并经发布方完成法律复核。

Phase 7A 会生成完整 dependency lock、CycloneDX/SPDX、许可证/版权清单、补丁包、对应源码归档和 PE 导入证据；这解决技术证据缺失，不等于法律意见。仓库内部清单继续保持 `blocked/false/false`，也不生成冒充已复核的 `THIRD-PARTY-NOTICES.md`。只有新候选技术矩阵通过、对应源码与实际二进制身份一致，并由发布方法律复核者批准通知和三项 audit 事实后，准备命令与 release/custom 构建才允许通过。

## 体积门禁

原始 `mpv.exe` 约 114 MiB，发布归档约 32 MiB；NSIS 压缩后预期使当前约 279 MB 安装包增加约 25–40 MB。最终安装包超过 320 MB 时必须先列出压缩后资源明细和移除方案。
