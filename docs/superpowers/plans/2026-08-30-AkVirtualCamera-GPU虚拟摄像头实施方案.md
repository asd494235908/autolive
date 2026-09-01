# AkVirtualCamera GPU 虚拟摄像头实施方案

> 状态：代码已接入·正式发布待门禁
> 更新日期：2026-09-01
> 目标平台：Windows 10 1903+ / Windows 11 x64
> 上游选择：[`webcamoid/akvirtualcamera`](https://github.com/webcamoid/akvirtualcamera)，固定提交、补丁和发布材料在实施阶段锁定

## 1. 决策摘要

桌面端虚拟摄像头唯一方案采用 AkVirtualCamera，不再并行维护 SoftCam、OBS Virtual Camera 或仅 Windows 11 可用的自研 `MFCreateVirtualCamera` 路线。产品安装一个名为 `GpAutoLive Camera` 的虚拟摄像头；用户在会议、浏览器、录制或直播软件的摄像头列表中选择该设备，GpAutoLive 自身只提供安装状态、启停、输出规格和故障修复，不把物理摄像头误写为可写目标。

GPU 是正式要求，但不把“使用 GPU”和“全程零拷贝”混为一谈：

- mpv `gpu-next/libplacebo` 继续在 D3D11/Vulkan 上完成 GPU83 实时效果并呈现最终画面。
- Windows Graphics Capture 从唯一最终效果窗口取得 D3D11 纹理，D3D11 Video Processor 或等价受管 GPU 路径完成缩放、裁剪和输出色彩转换。
- AkVirtualCamera 当前上游输入是 CPU 原始帧，随后通过共享内存或 socket 送入虚拟摄像头；首版因此允许在 GPU 处理完成后使用 staging texture 做一次异步、有界、可观测的 GPU→CPU 回读。
- 不允许重新在 CPU 上执行 GPU83、缩放或颜色效果，也不允许把 AkVirtualCamera 的原始帧入口描述成 D3D11 零拷贝。后续只有在真实扩展并验证共享 D3D11 纹理、同步原语和消费者兼容性后，才能宣称零拷贝。

这个边界是选择 AkVirtualCamera 后兼顾 Windows 10/11 和下游设备兼容性的最短闭环。若 Phase 0 证明一次回读在目标硬件上不能满足性能门禁，则停止产品接线，先评估 AkVirtualCamera D3D11 共享纹理扩展；不得静默降成全 CPU 视频链。

## 2. 已核实事实与准入风险

### 2.1 上游事实

- 2026-08-30 核对的上游主分支提交为 `9cf77ae6379e5f635255f4b377478d388a46a3b2`；正式实施仍需重新锁定审核通过的不可变提交。
- Windows 侧同时包含 DirectShow 与 Media Foundation 实现；Media Foundation 注册依赖 Windows 11 的 `MFCreateVirtualCamera`，Windows 10 兼容基线必须由 DirectShow 提供。
- `AkVCamManager stream` 从标准输入读取指定像素格式、宽高和帧率的完整原始帧；内部数据模式为共享内存或 socket。
- 当前源码未提供 D3D11、DXGI、`ID3D11Texture2D` 或共享纹理输入协议，因此原版不能提供 GPU 零拷贝闭环。
- 上游许可证为 GPL-3.0。任何修改、再分发和安装包交付都必须经过法律审核，并随包提供许可、版权、修改说明和完整对应源码/获取方式；未通过不得进入正式发布资源。

### 2.2 平台边界

- 最低系统为 Windows 10 1903（Build 18362），因为按 HWND 创建 Windows Graphics Capture 项从该版本开始可用。
- Windows 10：必须通过 AkVirtualCamera DirectShow 设备完成兼容验收；不得加载仅用于上游测试的伪 `mfsensorgroup.dll`。
- Windows 11：DirectShow 仍是默认正式端点。AkVirtualCamera 9.4.1 的 Media Foundation 组件上游仍标记 experimental 且默认关闭，必须单独通过 `MFCreateVirtualCamera`、`ActivateObject`、内存安全、目标应用和签名门禁后才能条件启用；首版不得把 MF 写成已交付能力。
- 首版只支持应用自身最终效果窗口，不支持任意桌面、任意窗口、物理摄像头转写或系统全屏采集。

## 3. 功能边界

### 3.1 目标

1. 将唯一最终效果窗口中已经真实呈现的画面输出为 `GpAutoLive Camera`。
2. 支持用户在 Chrome、Edge、Teams、Zoom、Discord、OBS 和 Windows Camera 等下游软件中选择该虚拟摄像头。
3. 首批输出规格固定为 `1280×720@30fps`，通过性能与兼容门禁后再开放 `1920×1080@30fps`；不在首版开放任意分辨率/帧率组合。
4. GPU 完成最终效果、缩放和颜色转换；CPU 只承担 AkVirtualCamera 上游接口要求的末端帧回读与投递。
5. 虚拟摄像头故障不改变本地播放、视频参数、音频输出、播放池或 RTMP 状态。

### 3.2 非目标

- 不向物理摄像头写画面，不枚举物理摄像头作为“输出设备”。
- 不接 OBS Virtual Camera，不要求用户安装 OBS。
- 不采集桌面或其他应用窗口，不录制、不上传、不经过 Go 控制面。
- 不输出虚拟麦克风；虚拟摄像头链只有视频。
- 不在首版承诺 HDR、4K、60fps、多虚拟摄像头实例或跨 GPU 零拷贝。
- 不把 CPU 全帧滤镜、FFmpeg 逐帧重编码或窗口截图轮询作为生产回退。

## 4. 目标数据链

```text
本地媒体
  → 常驻 mpv gpu-next/libplacebo（D3D11/Vulkan，GPU83）
  → final-effect HWND 的真实呈现结果
  → Windows Graphics Capture（ID3D11Texture2D）
  → D3D11 GPU 缩放/裁剪/色彩转换
  → 三槽 staging texture 异步回读（latest-wins）
  → AkVirtualCamera 原始帧生产者适配
  → AkVirtualCamera 共享内存
  → Windows 10/11 DirectShow（Windows 11 MF 仅实验门禁）
  → 用户选择 GpAutoLive Camera 的下游应用
```

必须输出“最终已经呈现的画面”，不能重新从源文件生成一套容易与当前 GPU83、seek、循环和降级状态漂移的第二视频链。Windows Graphics Capture 技术门禁必须先证明：窗口可见、被其他窗口遮挡、跨显示器、DPI 缩放和窗口重建时取得的纹理与用户看到的最终效果一致。若该门禁失败，再评估 mpv 原生渲染表面共享；不得改用 GDI/CPU 截图充数。

## 5. 运行时设计

### 5.1 单一所有者

新增一个 `VirtualCameraOutputManager`，独占以下资源：

- 配置 revision、session generation 和状态机；
- Windows Graphics Capture item、frame pool、D3D11 device/context；
- GPU 转换纹理、三槽 staging texture 和帧节拍器；
- AkVirtualCamera 生产者进程/连接、取消令牌、读取线程和 Join；
- 实际输出规格、累计投递/丢弃/回读超时、下游客户端摘要和结构化错误。

不为假想的第二后端创建通用虚拟摄像头 Trait。AkVirtualCamera 是唯一实现；未来真正引入第二实现时再提取接口。

### 5.2 状态机

```text
Unavailable → Installed → Starting → Ready → Streaming
                    ↘ Failed ← Recovering ←┘
Ready/Streaming/Failed → Stopping → Installed
```

- `Unavailable`：组件未安装、签名/架构/注册不匹配或系统版本不满足。
- `Installed`：设备已安装，但生产者未启动。
- `Starting`：创建 WGC、GPU 资源和 AkVirtualCamera 投递链。
- `Ready`：输出链已就绪，尚无下游客户端也属于正常状态。
- `Streaming`：至少一个下游客户端正在消费，帧投递持续推进。
- `Recovering`：窗口 HWND、D3D 设备或 AkVirtualCamera 连接发生暂态变化，执行有界重建。
- `Failed`：确定性错误或重建预算耗尽。失败保持本地播放，禁止无限重试。

### 5.3 背压和同步

- 捕获回调只把最新 D3D11 纹理及时间戳提交给容量 1 的 latest-wins 队列，不阻塞 mpv 或 WGC 回调。
- GPU 转换与 staging 使用三槽环；尚未完成的槽不得覆盖。没有空槽时丢弃当前虚拟摄像头帧并计数，不阻塞播放。
- 输出以单调时钟固定到 30fps。源帧更快时只取最新帧；源帧更慢时可以有界重复最新有效帧，暂停、停止、纯音频、锁屏或没有有效帧时固定输出黑帧，禁止长期保留上一段画面。
- 停止、切源、最终窗口重建、D3D 设备丢失和应用退出必须递增 generation；旧帧、旧回读和旧进程结果不得提交到新会话。
- AkVirtualCamera 生产者与任何辅助进程必须纳入 Windows Job Object；优雅停止有 deadline，超时终止并 Join。

### 5.4 GPU 真实性门禁

正式运行状态必须公开并可自动验证：

- `capture_api=windows_graphics_capture`；
- 实际 D3D11 adapter LUID、厂商、设备名称和 feature level；
- `gpu_scale=true`、`gpu_color_convert=true`；
- 回读次数、平均/P95/P99 回读耗时、队列丢帧和输出帧推进；
- AkVirtualCamera 投递模式、真实像素格式、分辨率、帧率与下游客户端数量。

以下任一情况不得显示“GPU 虚拟摄像头运行中”：WGC/D3D11 创建失败、转换改在 CPU、AkVirtualCamera 帧未推进、实际输出规格不匹配，或 GPU83 已因会话故障降到 CPU4/Original 但 UI 仍把画面描述为完整 GPU83。允许如实显示“GPU 捕获/转换 + Original 画面”，因为输出链 GPU 与效果能力是两个独立事实。

## 6. AkVirtualCamera 接入和安装

### 6.1 组件策略

- 上游源码、固定提交、项目补丁、构建脚本、许可证、版权和对应源码清单放入明确的 `desktop/third_party/akvirtualcamera/` 发布边界，不与 Rust 业务代码混合。
- 为降低 GPL 强 copyleft 与主程序直接链接的风险，首选独立 GPL sidecar/安装组件承接 `vcam_stream_send` 或上游 IPC，Rust 只通过受限本机控制通道管理它；最终边界仍以法律审核为准。不得在未审核前把 AkVirtualCamera C API 直接链接进闭源 Rust 主程序。
- 若标准输入额外复制成为瓶颈，可在独立 sidecar 内直接调用上游 C API/共享内存能力；不得复制整个 Manager 命令框架。AkVirtualCamera 9.4.1 的 C API 与 MF NV12 平面循环存在待核实的越界风险，修复并通过 ASan/边界测试前，NV12 不得进入生产；PoC 可用受控 YUY2/RGB 验证兼容性，但不能替代最终性能门禁。
- 上游 TCP 消息通道当前没有可接受的产品级认证与 ACL 证据，且源码存在非回环绑定风险。正式接入必须改为当前用户 ACL 的 Windows Named Pipe，或至少固定 `127.0.0.1`、增加每会话随机认证、拒绝远程连接；命名共享内存必须显式限制为当前用户、所需服务身份和 SYSTEM。未完成前属于发布阻断项。
- 安装器以管理员权限完成 x64/x86 DirectShow 注册、服务和默认占位图配置；Windows 11 MF 只作条件实验组件。32 位组件只用于让 32 位下游应用枚举同一设备，不把主桌面程序改成 32 位。
- 卸载必须先停止生产者、确认无活动客户端或明确提示，再注销组件并删除本产品创建的设备/服务；不得删除其他 AkVirtualCamera/Webcamoid 实例。

### 6.2 许可与签名门禁

- AkVirtualCamera 为 GPL-3.0；正式方案必须由法律审核确认与主程序的分发和进程边界、补丁发布、安装器聚合关系及对应源码义务。
- 所有 EXE、DLL、服务、驱动/过滤器和安装器必须 Authenticode 签名；未签名测试包只能内部验证。
- 资源清单必须锁定哈希、目标架构、上游提交、补丁摘要、SBOM、许可证和对应源码归档；缺一项 release/custom 构建 fail-closed。

## 7. UI 和用户流程

主窗口增加一个“虚拟摄像头”输出卡，首版只包含：

- 固定设备名 `GpAutoLive Camera`；
- 安装状态与“安装/修复”入口；
- 输出开关；
- `720p30`、通过门禁后可用的 `1080p30` 规格选择；
- 暂停/纯音频时的最后一帧或黑帧策略；
- 实际捕获 API、GPU、像素格式、帧率、客户端数量和脱敏错误。

用户真正的“选择摄像头”发生在目标应用中：从其摄像头列表选择 `GpAutoLive Camera`。GpAutoLive 不提供物理摄像头输出下拉框，也不以进程路径作为用户选择项。UI 草稿与活动配置分离；运行中修改只有保存并通过 Rust 校验后才原子重启虚拟摄像头 session。

## 8. 分阶段实施

### Phase 0：准入门禁

实施记录（2026-08-31）：已建立纯 Rust `autolive-virtual-camera-contract`，固定 YUY2 `1280×720@30fps`、`zero_copy=false`，并覆盖 GPU 真实性、generation、latest-wins、黑帧和回读百分位测试；已锁定 AkVirtualCamera 上游提交及 GPL 对应源码，加入发布材料校验器、受限 Tauri 状态/启动/停止入口、固定 sidecar 协议和当前用户令牌管道。`autolive-virtual-camera-native` 现采用 Windows Graphics Capture→D3D11 Video Processor→GPU BGRA→YUY2 着色器打包→三槽 staging 回读，CPU 只复制 staging 字节，不执行色彩转换；硬件不支持 YUY2 Video Processor 输出时仍可走 GPU pack。已修复 WGC 会话 WinRT 析构顺序，并兼容部分 Windows 11 构建返回的 `S_OK + null frame`。NSIS 仅在 `release-ready.json` 存在时注册 x86/x64 DirectShow 组件；未签名、缺少发布产物或未完成兼容矩阵时继续 fail-closed。

补充短测记录（2026-08-31）：同一真实桌面 HWND 的最新 3 秒基准通过，GPU 回读 P50/P95/P99 约为 3.1/12.2/12.2ms；该结果仅用于确认修复后的稳定退出和单帧预算，不替代长时、多硬件、Win10/11 与下游兼容门禁。

补充实现记录（2026-08-31）：桌面端 sidecar 资源路径统一为
`akvirtualcamera/bin/akvirtualcamera-sidecar-x64.exe`，与离线构建脚本和 NSIS 目录一致；
输出线程遇到最终效果窗口尺寸变化时最多重建两次 WGC/D3D11 捕获会话，并递增契约 generation，
期间向下游发送黑帧，不阻塞本地播放。窗口 HWND 被销毁时仍由最终效果关闭清理回收输出任务；
最终效果宿主重建后的跨 HWND 自动重绑代码已接入，但真实窗口销毁、跨 HWND 和实机下游恢复
仍待 Phase 1/3 门禁。启动命令每次重新核验
`release-ready.json` 及六个固定运行件；运行期间门禁失效时不沿用旧的 `Installed` 状态启动残留
sidecar。GPU 状态契约同时传递真实 adapter LUID、厂商 ID、设备 ID、名称和 Feature Level，
UI 以十六进制显示厂商/设备 ID，便于多 GPU 兼容矩阵核对。

补充验收记录（2026-08-31）：`cargo fmt --check`、桌面端 `cargo test --all-targets`
（共 590 个库测试及各集成目标）、Windows 目标 `cargo check`、虚拟摄像头 native
`clippy -D warnings`、资源/sidecar/NSIS Node 定向测试和 UI 生产构建均通过；测试包已在本机
构建出 `GpAutoLive Test_0.1.0_x64-setup.exe`。正式构建会先执行资源暂存和锁文件校验，当前因
缺少真实 DirectShow/Assistant/Manager/C API/sidecar/安装器签名产物、SBOM、法务审核、GPU
长测和 Win10/11 下游矩阵而 fail-closed，未把测试包标为可发布。

补充构建边界记录（2026-08-31）：`stage-akvirtualcamera-resources.mjs` 只把通过锁文件
校验的运行件和对外 GPL 材料复制到 Tauri 资源根，并在失败时撤销旧 `release-ready.json`；
法务、GPU 基准、兼容矩阵和签名证据留在发布证据目录。测试 profile 会清理该资源根下的
生成件，仅保留说明文件，确保测试安装包不会注册或伪装虚拟摄像头。编译脚本和 sidecar
CMake 选项变更后，`corresponding-source-manifest.json` 的 overlay 哈希及锁文件文档哈希
已同步，锁校验无哈希漂移。

补充本机构建记录（2026-08-31）：sidecar 的 MSVC 编译显式使用 `/utf-8`，避免中文源码
在简体中文代码页下被错误解析；sidecar/C API 通过本机安装布局冒烟，非法令牌退出码为 `2`，
有效令牌但未安装设备退出码为 `5`，未写入系统设备或注册表；同一安装布局中的
`AkVCamAssistant.exe`、`AkVCamManager.exe` 已完成 x64 构建，Manager 的只读
`devices`/`clients` 查询均退出 `0` 且当前无设备。临时 DirectShow、sidecar 和 C API
文件的 Authenticode 状态均为 `NotSigned`，因此继续留在临时目录，不进入发布资源。

补充状态闭环记录（2026-08-31）：sidecar 通过已加载的 `vcam_clients` C API 每 250ms
向父进程 stdout 发送脱敏的客户端数量，Rust 状态读取线程按 session generation 更新
`downstream_client_count`；只有数量大于 0 才进入 `Streaming`，数量回到 0 时恢复
`Ready`，避免把“帧已写入管道”误报成下游正在消费。UI 同步显示“未探测”与真实数量。
该改动已用本机 MSVC/CMake 重新构建 sidecar/C API（临时未签名 SHA-256 分别为
`619ea0d0e627a17b8b2d30cd31981de289fc3f71133f18dffedd51aebd4b13c6` 和
`8ea11a758b094580bf0ee56febc9676cd0fbc8066f5b5af83fb041f0aeee9cb0`），仍未复制到发布 artifacts。

补充本机临时编译记录（2026-08-31）：在 Windows 11 Build 26200、Visual Studio 17 2022、
Windows SDK 10.0.26100.0 和 CMake 生成器下，离线脚本已分别完成 x64/x86 DirectShow
以及 x64 sidecar/C API/Manager/Assistant 编译。临时产物哈希为：x64 DirectShow
`dc468e1897cbd7a1b9bb4bde15e9f58eac143adbb64d9210b76bdd4a939b5ae3`、x86 DirectShow
`aece2e807bbd711fee12050e6152bc533b4973752a0b7275b57e2700e85f0d80`、sidecar
`6cb7461a23fc60359065f546841b04355abb0435fbe267cbcb540e58f661b378`、C API
`140c1b0dc19c55e7c06f836b61ee6361a77d81f974397c4cac1dd96fff8f3e20`、Manager
`9e6858d2eef4b7bd3a5956e29efa27d5f9c4622a9121cabc64dfecc352936ff1`、Assistant
`855570014163934f5a5c5ab67df1ca26169269c8a412441970e6a08922f95fd4`。所有文件均为
`signed=false` 的临时构建，未复制到锁定 artifacts，未生成 `release-ready.json`，也未
注册系统设备；`dumpbin /headers` 已确认 DirectShow 分别为 PE machine `8664`（x64）和
`14C`（x86），签名和实机安装门禁仍必须单独完成。

补充构建幂等记录（2026-09-01）：`build-directshow.ps1` 和 `build-sidecar.ps1` 的
loopback 安全补丁现在先执行 `git apply --check`，若同一 `BuildRoot` 已经应用则改为
`--reverse --check` 验证，不会二次修改源码；补丁处于不一致状态时 fail-closed。使用
同一临时构建根重复运行 x64 DirectShow，首次和第二次均退出 `0`，确认该路径可重复
执行。设备验收脚本同时拒绝相对 `InstallRoot`，避免把验收根解析到未预期目录。

补充窗口重绑记录（2026-08-31）：最终效果宿主重建后，桌面端比较当前捕获会话绑定的
HWND；只有句柄实际变化时才进入 `Recovering`，回收旧 WGC/D3D11/sidecar 并以新句柄
建立单一新会话。重绑失败只把虚拟摄像头置为 `Failed`，不停止 mpv、播放池或最终效果
窗口；相同 HWND 不重复重启。真实窗口销毁、跨 HWND 和下游恢复仍需 Win10/11 实机门禁。

补充发布证据记录（2026-08-31）：已生成并锁定 CycloneDX SBOM、法务审查记录、
720p30 基准 JSON 和 Win10/11 下游兼容矩阵 JSON。SBOM 已包含锁定的上游提交和 GPL
许可证；法务记录明确为 `pending`，基准仅为 3 秒短测，兼容矩阵全部为 `not_run`。
锁校验器现在会读取这些证据内容，只有法务 `approved`、至少 7200 秒 GPU 基准和
全部兼容条目 `passed` 才能进入发布状态，不能用占位文件绕过门禁。

补充签名工具记录（2026-08-31）：新增 `sign-akvirtualcamera-artifacts.ps1`，只读取锁定
的 7 个产物，强制检查当前用户证书库中的代码签名 EKU、私钥和有效期，使用 SHA-256
Authenticode 签名及 `signtool verify /pa /all`，再原子写出逐产物证据 JSON。当前机虽有
Windows SDK `signtool.exe`，但没有有效代码签名证书，脚本按设计 fail-closed；未生成
自签名或占位发布件。

补充设备验收入口记录（2026-08-31）：新增 `verify-akvirtualcamera-device.ps1`，只读
检查 `GpAutoLive Camera` 的 PnP/DirectShow 可见性、x86/x64 注册表所有者、固定组件
路径和 `AkVCamManager devices` 输出；不调用 `regsvr32`、不添加/删除设备、不终止
其他进程。CPU test-pattern、下游应用和退出/崩溃/卸载清理保持显式 `not_run`，未完成
时返回退出码 `2`。本机实测报告为 Windows 11 Build 26200、未找到设备，故保持阻断。

补充安全边界记录（2026-09-01）：sidecar 会话令牌不再出现在命令行参数或环境变量中。
Rust 父进程以继承的 stdin 一次性写入 32 位十六进制令牌并立即关闭管道，sidecar 仅接受
`--session-token-stdin` 后读取并校验令牌，再按当前用户 ACL 创建 Named Pipe；已有的
`PIPE_REJECT_REMOTE_CLIENTS`、帧头/代际/序列/尺寸/长度校验保持不变。stdin 缺失、写入
失败或 sidecar 提前退出均走有界 kill+wait，不影响 mpv、本地播放和其他音频链。对应源码
清单及 `upstream.lock.json` 文档哈希已同步，静态 sidecar/锁/签名/设备测试 11 项通过，
隔离 Cargo target 的虚拟摄像头契约集成测试 3 项通过。该改动只收紧会话认证传输，不改变
GPL、DirectShow 默认端点、GPU 捕获/转换或发布门禁状态。

补充本机安全协议构建记录（2026-09-01）：使用 Visual Studio 17 2022、Windows SDK
10.0.26100.0 和 CMake 离线重建 x64 sidecar/C API；sidecar SHA-256 为
`d7b57211e9eef04b725dff2434ede69c511c18d5352a038e48dbf4a8799ce778`，配套 C API 为
`9db55460dcad6903ee2cc0afc5398332fadde0733c45c112bedb8a87e3c323aa`。通过
`System.Diagnostics.Process` 将合法令牌写入重定向 stdin，未安装设备时按预期退出码 `5`；
非法 stdin 令牌按预期退出码 `2`，stdout/stderr 均无敏感令牌。该构建仍为未签名临时产物，
未复制到正式 artifacts、未注册设备或修改系统注册表。

补充 GPU 基准门禁记录（2026-09-01）：技术样例新增至少 95% 目标帧覆盖率和
interframe P95 ≤ `50,000µs` 的帧推进判定，并将 `minimumFrames`、`framesDelivered`、
`frameCadenceWithinBudget` 写入 JSON；锁校验器要求这些字段同时通过。这样“只捕获到一帧”
或静止窗口不会被误报为 720p30，仍需在真实视频、真实 GPU 和连续 7200 秒环境运行后才能
填充正式证据。

补充状态通道边界记录（2026-09-01）：Rust 状态读取线程改用固定上限的逐字节读取器，
超出 64 字节的 sidecar stdout 行会被丢弃到换行符且不会扩容无界缓冲，随后仍能继续解析
下一条 `GPAKVC_CLIENTS` 状态；新增超长行恢复测试通过。这样即使 sidecar 组件损坏或输出
异常，也不会让状态通道把内存或协议边界拖垮。

补充 Windows 脚本兼容记录（2026-09-01）：四个 AkVirtualCamera PowerShell 入口统一
写入 UTF-8 BOM，修复 Windows PowerShell 5.1 在无 BOM UTF-8 中文字符串后吞掉引号、导致
设备验收脚本解析失败的问题。已用系统 `powershell.exe` 5.1 对构建、签名和设备验收入口
逐一执行参数解析；设备验收在当前无设备环境按预期返回退出码 `2` 并输出阻断报告。

补充 GPU 采样映射修复记录（2026-09-01）：复核 YUY2 打包像素着色器的全屏三角形
插值。打包纹理宽度为源画面的 1/2，每个目标 texel 承载两个源像素；目标渲染边界需要
覆盖源纹理归一化坐标 `0..1`，因此 overscan 顶点保留 `float2(2.0,1.0)`，让三角形在
目标视口内正确内插到完整 `0..1`，不会只覆盖源画面半宽。修复后仍由 GPU 完成
BGRA→YUY2 打包，CPU 只回读 staging 字节；新增 Windows 原生 crate 静态着色器契约
测试，9 项单元测试和 `clippy --all-targets -D warnings` 均通过。

补充真实最终窗口短测记录（2026-09-01）：在桌面端播放用户提供的
`D:\xz\e91e31b67264b3d24eb929b8e910717c.mp4`（源素材 `26fps`）并以最终效果 HWND
执行 30 秒 WGC→D3D11→YUY2 staging 测试，收到 778 帧，GPU→CPU 回读 P99 为
`5,330µs`，低于 `33,333µs` 单帧预算，时间戳单调且 GPU 缩放/色彩转换路径生效。
该样例的帧推进覆盖门禁按目标 30fps 要求至少 855 帧，因此因输入源只有 26fps 而
保持失败；这不是回读性能失败，也不能作为正式 7200 秒发布证据。运行时 sidecar
仍按固定 `30fps` 时钟重复发送最新有效帧；正式证据需使用真实 30fps 动态源并连续
运行至少 7200 秒。

补充本地静态门禁回归（2026-09-01）：AkVirtualCamera 相关锁校验、资源 staging、
DirectShow/sidecar 构建入口、NSIS、设备验收和虚拟摄像头 UI 共 16 项 Node 测试
全部通过；当前生产锁仍明确报告 `releaseReady=false`，未把测试夹具写入正式资源。

补充设备丢失恢复记录（2026-09-01）：输出线程现在把 D3D11 `DeviceLost` 与 WGC
`FrameSizeChanged` 视为独立的有界恢复事件，最多重建两次 WGC/D3D11 捕获会话；重建前
进入 `Recovering`、递增契约 generation 并先向 sidecar 发送固定黑帧，成功后复用已验证
的 GPU 事实回到 `Ready`。超过上限或缺少已验证 GPU 事实时才进入 `Failed`，不会停止 mpv、
本地播放池或声音链。桌面端 `cargo test --all-targets`（599 个库测试、110 个主程序
测试及全部集成目标）和 AkVirtualCamera 定向 Node 16 项测试均通过；该恢复路径仍需在
真实 D3D 设备重置和窗口缩放场景中完成 Phase 3 实机验证。

补充前端全量回归记录（2026-09-01）：`pnpm --dir desktop/ui test` 的 AkVirtualCamera
相关测试保持通过；全量套件另有 3 项既有 Phase 7A mpv FriBidi 固定输入测试失败，
原因是仓库现有 `fribidi-native-compiler.patch` 与锁定尺寸/内容不一致。本次虚拟摄像头
改动未触碰该补丁，未将这 3 项失败误报为 AkVirtualCamera 回归，也未擅自修改无关 mpv
输入锁。

补充实际 GPU 事实记录（2026-09-01）：`NativeCapturePump` 启动握手现在返回实际
WGC/D3D11 捕获会话所选 adapter 的 LUID、名称、VendorId、DeviceId 和 Feature Level；
桌面状态与 UI 使用该运行时事实，不再把仅用于准入预检的独立 adapter probe 当作实际
捕获 GPU。D3D11 设备丢失或 WGC 帧尺寸变化后的有界重建也会重新读取新会话事实，再更新
`Ready` 状态；若会话未返回事实则保持 fail-closed。对应映射和运行时使用路径已加入 Rust
单元测试，桌面端全量 Cargo 测试（599 个库测试、111 个主程序测试及全部集成目标）和
原生虚拟摄像头 crate 的 9 项测试、两套 `clippy --all-targets -D warnings` 均通过。

补充停止/重启收敛记录（2026-09-01）：虚拟摄像头运行时停止现在无论旧 task 的
`stop/join` 是否返回错误，都会继续执行 `VirtualCameraOutputManager::stop`，并返回
首个错误，避免状态停留在 `Streaming/Starting`。启动替换旧 task 失败时也会明确记录
`Failed` 原因，避免下一次请求被误判为“start in progress”；运行时锁在阻塞停止期间
不再被持有。新增契约回归测试覆盖该顺序和失败收敛，完整桌面 Cargo 测试（599 个库
测试、111 个主程序测试及全部集成目标）与 `clippy --all-targets -D warnings` 均通过。

补充当前 Windows 设备验收记录（2026-09-01）：在本机 Windows 11 专业版 Build 26200
只读运行 `verify-akvirtualcamera-device.ps1`，结果为 `status=blocked`、退出码 2；未发现
正式签名组件的 x86/x64 注册表所有者、PnP/DirectShow `GpAutoLive Camera` 设备或
`AkVCamManager.exe`，CPU test-pattern、下游应用和退出/崩溃/卸载/客户端占用证据均为
`not_run`。该结果证明当前环境没有可误报的已安装设备，不能替代正式安装包和实机矩阵。

补充运行时槽位异常记录（2026-09-01）：启动或 HWND 重绑在新 task 已经创建后，若运行时
槽位锁损坏，统一提交入口会先停止该 task（包含 sidecar、捕获线程和 Job Object），再将
manager 置为 `Failed` 并返回错误；不再留下无主线程或永久 `Starting/Recovering` 状态。新增
契约测试覆盖该所有权转移路径；定向 `virtual_camera_contract` 4 项测试和桌面端
`clippy --all-targets -D warnings` 均通过。

补充 CPU test-pattern 工具记录（2026-09-01）：新增
`desktop/third_party/akvirtualcamera/run-test-pattern.ps1`，只启动指定的独立 sidecar，
通过当前用户 Named Pipe 发送 CPU 生成的 YUY2 `1280×720@30fps` 帧，并输出可传给设备
验收脚本的 JSON（帧数、序列单调性、sidecar 退出码和脱敏输出）。工具不注册、卸载或
修改系统设备；当前未安装正式设备时实测保持 `blocked`，不会被误用为兼容性通过证据。
对应静态测试已通过，Windows PowerShell 5 UTF-8 BOM 和绝对路径门禁已验证。

补充 stdin 兼容修复记录（2026-09-01）：实测 Windows PowerShell 5 的重定向输入会在
令牌前自动写入 UTF-8 BOM，旧解析器因此直接返回退出码 2。sidecar 现在只额外接受规范
`EF BB BF` 前缀，随后仍严格校验 32 位 ASCII 十六进制令牌；使用新构建的 sidecar
在 PowerShell 5 下已进入设备初始化路径并因本机未安装设备返回退出码 5，证明令牌不再
被误判。对应 C++ overlay 哈希、修改说明和 sidecar 静态测试已同步更新。

补充发布清单同步记录（2026-09-01）：stdin 兼容补丁变更后已重新计算
`MODIFICATIONS.md`、`corresponding-source-manifest.json` 及 sidecar 源码 SHA-256，
并写回 `upstream.lock.json`；发布校验器恢复为仅报告真实缺失产物/签名/法务/长测/矩阵
阻断，不再报告文档哈希漂移。

补充 sidecar 本机构建记录（2026-09-01）：使用本机 Visual Studio 2022 Build Tools、
Windows SDK 和 `/utf-8` 选项重新编译 stdin 兼容修复后的 x64 sidecar，临时未签名 SHA-256
为 `10a171473eb5c566187c9ec8adb2015c188dc16b50424add644fc0513b328558`。在同目录放置
临时 `vcam_capi.dll` 后，PowerShell 5 test-pattern 已通过令牌解析并进入设备初始化，
因无系统设备而返回 `blocked`，产物未进入发布 artifacts、未注册设备。

补充 AkVCam 权限边界修复记录（2026-09-01）：核对锁定上游 C API 源码后确认
`vcam_set_data_mode` 与 `vcam_set_direct_mode` 在 Windows 上属于需要提升权限的
配置入口；sidecar 若在每次启动调用，会触发 UAC 并破坏自动恢复。安装器现于提升阶段
一次性执行 `set-data-mode mmap` 与 `set-direct-mode`，sidecar 改为只读校验
`vcam_data_mode`/`vcam_direct_mode` 后再启动流，保持运行时当前用户权限。已用本机
MSVC/CMake 重新编译临时 x64 sidecar（SHA-256
`a8477670dad94d2b55418da3a469e575c8c7bb9a0d6137f1837265a7d25de151`），无设备时
test-pattern 仍按预期返回 `blocked`；对应源码清单、锁文件和静态测试已同步。

补充 UI 事实可见性记录（2026-09-01）：虚拟摄像头输出卡现在分别展示运行时返回的
`capture_api`、实际 adapter LUID/厂商/设备/Feature Level，以及 `gpu_scale`、
`gpu_color_convert` 两项 GPU 转换事实；未建立捕获会话时继续显示等待准入，不以预检
结果冒充运行时 GPU。对应 UI 静态测试、TypeScript 类型检查和生产构建均通过。

补充 benchmark 可执行文件校验记录（2026-09-01）：发现工作区遗留的旧 benchmark
二进制会把“只收到 1 帧”错误标记为 `passed`；已在隔离 Cargo target 中按当前源码重新
编译 `virtual_camera_gpu_benchmark`，对当前最终效果 HWND 运行 10 秒后正确输出
`status=failed`、`minimumFrames=285`、`framesDelivered=1`、`frameCadenceWithinBudget=false`，
并以非零退出码结束。旧二进制报告不再作为任何门禁证据；正式 7200 秒证据必须使用
当前源码重新构建且接入真实 30fps 动态画面。

续测记录（2026-09-01）：使用当前桌面主窗口 HWND 的 10 秒短测同样严格失败（`framesDelivered=4`、
`sequenceAdvances=4`、`p99ReadbackWithinBudget=true` 但 `frameCadenceWithinBudget=false`），
说明静态/非最终效果窗口不能被当作 30fps 动态证据；该报告仅保留为 fail-closed 诊断，未写入
正式 GPU 长测文档。

补充卸载占用提示记录（2026-09-01）：NSIS 卸载钩子在确认注册表所有权属于本产品后，
先提示用户退出 GpAutoLive 生产者及 Chrome、Teams、Zoom、OBS 等摄像头下游；用户选择
取消会立即 `Abort`，不会强杀其他进程。确认后才继续移除固定设备、注销 x86/x64
DirectShow 过滤器和恢复注册表，满足“先停止生产者/明确提示占用”的卸载边界。

补充安装回滚记录（2026-09-01）：NSIS 在调用 x86/x64 `regsvr32` 前先记录待回滚状态，
即使注册工具只完成部分注册后返回错误，失败路径也会尝试注销对应过滤器，再恢复注册表
并移除本次新增设备，避免安装失败留下半注册组件。

补充生命周期串行化记录（2026-09-01）：桌面端为虚拟摄像头启停、最终效果 HWND 重绑和
退出清理增加同一生命周期锁；并发命令不会交错推进 `Starting/Recovering`、重复创建
sidecar 或在旧任务回收前提交新任务。新增集成静态门禁覆盖启动、停止和重绑均持有该锁，
`virtual_camera_contract` 5 项测试通过。

补充 IPC 状态校验记录（2026-09-01）：桌面 React 在接收虚拟摄像头 Tauri 状态时，
现在固定校验状态枚举、设备名、YUY2 1280×720@30fps、`zero_copy=false`、GPU 捕获/转换
事实、下游客户端数上限、帧计数关系和有界错误字符串；异常或漂移响应被拒绝，不会以
不可信 IPC 数据驱动 UI 或按钮状态。对应虚拟摄像头 UI 静态测试、TypeScript 类型检查
和生产构建通过。

补充原生 GPU 准入边界记录（2026-09-01）：`CaptureConfig` 现在在创建 WGC/D3D11
线程前就拒绝非首版固定 `1280×720` 输出，避免到 GPU YUY2 打包阶段才失败；adapter
预检同时创建 BGRA 输出面、三槽 staging 和 YUY2 打包资源，只选择能够完成整条 GPU
转换链的硬件 adapter。D3D11 `DEVICE_REMOVED`、`DEVICE_HUNG`、`DEVICE_RESET` 和
`DRIVER_INTERNAL_ERROR` 均归类为 `DeviceLost`，进入既有有界重建路径；新增固定规格
负向单测，原生 crate 9 项测试与 `clippy --all-targets -D warnings` 已通过。

补充退出与固定帧率边界记录（2026-09-01）：输出任务停止时先设置取消标志并等待 sidecar
最多 500ms 的优雅退出窗口，超时才终止进程并 Join；状态读取线程沿用更短的终止窗口，
避免异常 sidecar 或写管道背压造成无界等待。原生 `CaptureConfig` 同时在创建 WGC/D3D11
线程前拒绝非 `30fps` 配置；Video Processor 提交无论成功或失败都会释放输入视图，避免
驱动拒绝提交时遗留 COM 资源。对应原生测试、桌面端 check/clippy 均通过。

补充 CPU test-pattern 变化帧记录（2026-09-01）：`run-test-pattern.ps1` 预计算 8 帧
移动彩条 YUY2 样本并按 30fps 环形发送，避免在发送循环内重复生成约 1.8 MiB 帧；该工具
仍只用于已安装设备的原始帧/下游验收，不会把变化帧脚本或临时 sidecar 写入正式发布资源。

补充全量回归记录（2026-09-01）：在隔离 Cargo target 中完成桌面端 `cargo test --all-targets`，
库测试 599 项、主程序测试 111 项及全部集成目标均通过；`cargo clippy --bin
autolive-desktop-core --all-targets -- -D warnings`、UI `tsc --noEmit` 和 AkVirtualCamera
相关 Node 18 项静态测试均通过。`verify-akvirtualcamera-lock.mjs --require-release-ready`
仍以退出码 2 报告真实发布阻断，不把临时未签名构建或未安装设备写入 release-ready。

本轮定向回归（2026-09-01）：桌面端 `cargo check --bin autolive-desktop-core`、
`cargo clippy --bin autolive-desktop-core --all-targets -- -D warnings`、虚拟摄像头
集成测试 4 项，以及 AkVirtualCamera 锁/sidecar/安装器/设备/UI 静态测试 12 项均通过。
本机设备验收仍如实输出 `blocked`：Windows 11 Build 26200 未安装正式签名组件，注册表
所有者、PnP、DirectShow、Manager、CPU test-pattern、下游和清理证据均为空或未运行；
未将该结果写成发布通过。

补充启动取消回归（2026-09-01）：sidecar 已连接管道但未发送激活信号时，输出线程改为
以 50ms 有界轮询同时观察取消标志；停止操作不再被 5 秒激活等待拖住，正常激活仍受统一
启动超时约束。新增取消/成功信号单测，虚拟摄像头集成测试 5 项通过，桌面端 `clippy`
在 `-D warnings` 下通过。

补充令牌边界回归（2026-09-01）：Rust `validate_pipe_name` 与 sidecar 的会话令牌校验
统一拒绝全零 128-bit 令牌，避免固定管道名被误当成随机会话端点；原生 crate 9 项测试及
`clippy --all-targets -D warnings` 通过。

补充 GPU 基准证据完整性（2026-09-01）：`virtual_camera_gpu_benchmark` 现在把实际捕获
会话返回的 adapter LUID、名称、VendorId、DeviceId 和 Feature Level 写入报告，并每秒对
benchmark 进程采样工作集与虚拟内存。发布校验器要求 7200 秒报告至少包含两个资源样本，
工作集峰值增长不超过 64 MiB、虚拟内存峰值增长不超过 256 MiB，且 `growthWithinBudget=true`；
缺少真实 GPU 事实或资源采样时保持 fail-closed。该改动只增强证据，不把当前短测或静态窗口
报告升级为正式长测。

本机证据复测（2026-09-01）：使用当前源码重新构建 benchmark，在本机最终效果宿主 HWND
运行 3 秒，报告记录实际 AMD Radeon RX 6750 GRE 10GB（VendorId `4098`、DeviceId
`29695`、Feature Level `0xb000`），资源采样 4 次且峰值增长在预算内；由于该窗口没有持续
推进 30fps 动态画面，仅收到 2 帧，进程按设计返回失败。该结果只证明 GPU 事实与资源采样
路径有效，不能替代 7200 秒动态源门禁。

全量回归复核（2026-09-01）：针对令牌边界修复重新执行桌面端 `cargo test --all-targets`
（库 599 项、主程序 111 项及全部集成目标）和 `virtual_camera_contract` 5 项，均通过；
`cargo fmt --all -- --check`、原生 crate `clippy --all-targets -D warnings`、UI 类型检查
和 AkVirtualCamera Node 18 静态测试 18 项均通过。正式发布锁与本机设备验收仍保持
`releaseReady=false`/`blocked`，没有把未签名临时产物写入发布树。

动态源复测（2026-09-01）：对当前桌面实例识别到的最终效果窗口执行 10 秒当前源码
benchmark，因该窗口在测量期间没有持续推进动态画面，仅收到 1 帧，严格返回非零退出码
并标记 `frameCadenceWithinBudget=false`；该诊断再次证明静态窗口不能充当 7200 秒 GPU
长测证据，报告未覆盖正式 `gpu-benchmark-720p30.json`。

全量串行回归（2026-09-01）：并行 `cargo test --all-targets` 首次在 Windows 链接阶段因
同时启动多个 linker 触发 LNK1102 内存不足；随后使用隔离 Cargo target 和 `-j 1` 重新执行，
库测试 599 项、主程序测试 113 项、全部集成目标和 benchmark 示例 6 项均通过。相同隔离
target 上 `cargo clippy --bin autolive-desktop-core --all-targets -j 1 -- -D warnings` 通过；
AkVirtualCamera 相关 Node 静态测试 18 项、UI 类型检查和 `cargo fmt --all -- --check` 亦通过。
该结果只证明当前源码回归稳定，不改变真实签名、设备、下游矩阵和 7200 秒长测的发布阻断。

GPU YUY2 UV 映射复核（2026-09-01）：代码总监复核全屏三角形几何后修正 `GpuYuy2Pack`
overscan 顶点的源纹理 UV，第三个顶点使用 `float2(2.0, 1.0)`，确保目标边界完整覆盖
源纹理归一化坐标 `0..1`，避免只采样源画面半宽；补充原生 crate 静态测试并通过 9 项测试。
修复保持 GPU 色彩转换与单次有界 staging 回读边界，不改变发布门禁或设备安全边界。

补充离线构建证据（2026-09-01）：使用本机 Visual Studio 17 2022、Windows SDK
10.0.26100.0、CMake `Visual Studio 17 2022` 生成器和锁定上游归档，在隔离临时目录
分别完成 x64 sidecar/C API/Assistant/Manager 及 x64、x86 DirectShow 构建；构建网络为
`none`，loopback 补丁通过 `git apply --check` 后应用。临时文件 SHA-256 为：sidecar
`04b15b1f74b60ec0cf2ded7fe21b699c189e9ec282a0151ba70885933cd94ad4`、C API
`60a946b678fdda2ff66325a87af8801b332b6ad8e193c83b7ae2db9bc86fcf21`、Assistant
`815902eb7a9d820e8f631ccb3589059f561451a4c5dfa3309d798ae55493803c`、Manager
`e86bcd67c497940a1cca6b9ecaf9f24861ed9acd2ac394bde90ae488c3796085`、x64 DirectShow
`a17330074f400dbce40d5bfe3abb3ae91bb426e688622fcc9dc324c0ede33635`、x86 DirectShow
`7367962689242bba599b5162646d5520729ebcb499553fe71d219118a9eee439`。产物均为
`signed=false`，仅保留在 `tmp-akvcam-*` 临时目录，未复制到正式 artifacts、未生成
`release-ready.json`，未安装或注册系统设备；安装器仍需真实签名产物和实机门禁。

补充 sidecar 冒烟记录（2026-09-01）：使用上述临时 x64 sidecar 运行
`run-test-pattern.ps1 -Seconds 1`，令牌管道连接超时后 sidecar 返回退出码 `5`，证据为
`status=blocked`、`frames=0`；原因是本机没有已安装的 AkVirtualCamera 设备。该结果验证
了新构建的令牌/管道和“无设备不发送帧”的 fail-closed 行为，不代表 DirectShow 注册或
下游兼容性通过。

三槽异步回读闭环修复（2026-09-01）：复核发现旧实现虽然创建三个 staging 槽，却在任一
槽 pending 时停止消费 WGC，实际退化为单槽串行。现改为有空槽就继续提交新帧、同一轮轮询
全部已完成槽并只交付最大序列号，同时记录 `last_delivered_sequence` 丢弃乱序旧帧；三槽
仍保持有界，满槽时只返回已完成结果而不阻塞最终效果窗口。桌面端全量 `cargo test
--all-targets -j 1` 及原生 crate 9 项测试通过。

动态最终效果短测（2026-09-01）：导入用户提供的 `e91e31b67264b3d24eb929b8e910717c.mp4`
并同时打开视频/声音处理，点击播放后对真实 `GpAutoLive 最终效果` HWND 运行当前源码
10 秒 benchmark。实际 GPU 为 AMD Radeon RX 6750 GRE 10GB，收到 260 帧，GPU 回读
P99 为 4.688 ms，资源峰值增长 8.4 MiB（工作集）/2.6 MiB（虚拟内存），时间戳单调；
报告仍严格失败，因为源素材原始帧率为 26fps，未达到 720p30 门禁的 95% 帧覆盖及 50ms
interframe P95（实际 55.555ms）。该结果证明三槽链路在真实最终窗口上持续工作，但不能
替代使用 30fps 动态源的 7200 秒正式证据。

30fps 开发短测（2026-09-01）：以同一用户素材离线生成仅用于测试的 20 秒 30fps 副本，
在桌面端开启视频/声音处理并对真实最终效果 HWND 运行 60 秒当前源码 benchmark。实际
收到 1,758 帧（最低 1,710），interframe P95 48.611ms，GPU 回读 P99 4.353ms，时间戳
单调；工作集峰值增长 6.8 MiB、虚拟内存峰值增长 8 KiB，`growthWithinBudget=true`，
短测报告 `status=passed`。该副本和 60 秒报告仅作为本机开发证据，正式门禁仍要求
原始动态源覆盖 7,200 秒、多 GPU/Win10/11 及下游应用矩阵。

7200 秒正式 GPU 长测（2026-09-01）：在同一真实最终效果 HWND 使用当前源码、AMD
Radeon RX 6750 GRE 10GB（VendorId `4098`、DeviceId `29695`、Feature Level `0xb000`）
运行 `virtual_camera_gpu_benchmark --seconds 7200`，实际耗时 7,200,120ms，收到
210,824 帧（门槛 205,200），时间戳单调，帧覆盖与 cadence 门禁通过；interframe P95
48.611ms，GPU→CPU staging 回读 P99 8.104ms，工作集峰值增长 12.2 MiB、虚拟内存峰值
增长 672 KiB，`growthWithinBudget=true`。报告已写入
`desktop/third_party/akvirtualcamera/gpu-benchmark-720p30.json`，SHA-256
`8c25409ed01c6416bf05bac182c1852ac4d48a4d25c13d8013bfe8639f477b05` 并更新锁文件。
该证据只覆盖当前 AMD/Windows 主机；签名、法律、Win10/11、多 GPU、下游和清理矩阵仍
保持发布阻断，报告 `releaseReady=false`。

长测后设备复核（2026-09-01）：重新运行 `verify-akvirtualcamera-device.ps1`，当前 Windows
11 Pro build 26200 仍未发现 x64/x86 注册表所有者、PnP 设备、DirectShow 端点或
`AkVCamManager.exe`，设备验收保持 `status=blocked`；未把 WGC/D3D11 窗口长测结果误记为
虚拟摄像头安装或下游兼容通过。

- [x] 锁定 AkVirtualCamera 提交并保存 GPL-3.0 对应源码、修改说明和分发边界；法律审核仍待完成。
- [x] 本机构建上游 DirectShow x86/x64；2026-08-31 已通过本机 Visual Studio 2022 Build Tools/CMake 离线构建并输出未签名临时产物（x86 SHA-256 `5d0b08c0a5d8d4e02fc10255a805a34ed344c0001ad43f3fb55424e248d953f5`，x64 SHA-256 `493c7099a8ea3462270c3d57cdf8093569b19c818c81f694bc95521eb75c9a50`）。这些产物未复制到发布 artifacts、未写入锁文件，也未标记为可发布；签名、安装/注册和卸载门禁仍待完成。Windows 11 仅把 Media Foundation 作为实验门禁验证。禁止把源码放到服务器构建。
- [x] 本机构建独立 x64 sidecar/C API；2026-08-31 已通过本机 Visual Studio 2022 Build Tools/CMake 离线构建并输出未签名临时产物（sidecar SHA-256 `32a678a16b1a7ee4a1c808a368706c15970cb519c77a4a46dfc0a056f9283fa0`，sidecar 配套 C API SHA-256 `64fdced8e4d732afcbff1a901a299f12ee87c2bde65ce55da7c30383b884e0b`）。临时输出仍不进入发布资源树，签名、法律和设备门禁仍待完成。
- [ ] 用上游 test pattern/CPU 原始帧仅验证设备注册、枚举、卸载和下游兼容性；该结果不代表产品 GPU 链完成。
- [x] 建立 WGC HWND → D3D11 texture 技术样例，证明能捕获真实 mpv/libplacebo 最终效果。
- [x] 完成 D3D11 GPU 转换 + 三槽异步回读基准；当前 AMD/Windows 主机已通过 7200 秒动态源长测，
  多 GPU、Win10/11 和下游矩阵仍待完成，未把单机长测扩大解释为全平台兼容。

### Phase 1：最小 GPU 闭环

- [x] 新增 `VirtualCameraOutputManager`、状态机、取消、超时、generation 和资源释放测试。
- [x] 完成 WGC、D3D11 GPU 缩放/色彩转换、三槽 staging 和容量 1 latest-wins 捕获泵的原生边界；GPU 捕获只允许最终效果 HWND。
- [x] 接入独立 AkVirtualCamera sidecar 原始帧投递，固定 `720p30`；无 C API/设备或未通过发布门禁时 fail-closed。
- [x] 最终窗口重建时仅在 HWND 变化后重绑单一 WGC/D3D11/sidecar 会话，不创建第二播放器、不改变本地播放；暂停、seek、循环、切源、纯音频和处理开关仍沿用既有单播放器链路。真实窗口销毁、跨 HWND 和下游恢复需继续通过实机门禁。

### Phase 2：产品 UI 与安装

- [x] 添加最小输出卡、安装/修复、开关、规格、实际状态和错误；使用 Ant Design，不新增重复组件；在发布组件/签名缺失时安装入口保持 fail-closed。
- [x] NSIS 已增加精确组件安装/卸载、x86/x64 DirectShow 注册、注册表所有权检查、设备级回滚和跨产品保护；Windows 11 MF 仍保持条件实验门禁，实际签名发布资源和实机安装/卸载验收待完成。
- [x] 增加 GPL 文案、修改声明、CycloneDX SBOM、完整对应源码清单和固定哈希/内容级发布门禁；法务批准本身仍由上方独立门禁控制。

### Phase 3：兼容性与性能

- [ ] 通过 Windows 10/11、Intel/NVIDIA/AMD、多显示器、100%/125%/150% DPI 和 D3D 设备重置矩阵。
- [ ] 下游覆盖 Chrome、Edge、Teams、Zoom、Discord、OBS、Windows Camera，以及至少一个 32 位 DirectShow 客户端。
- [x] 当前 AMD/Windows 主机 720p30 连续 2 小时无资源增长；多 GPU、Win10/11 仍待完成，
  1080p30 只有在同等门禁通过后才开放。
- [ ] 应用退出、崩溃、卸载和客户端占用场景无残留生产者、服务、线程、句柄或错误注册项。

## 9. 验收标准

1. Windows 10/11 下游应用能枚举并选择 `GpAutoLive Camera`，输出与最终效果窗口一致。
2. GPU83、CPU4、Original 三种真实画面状态均能按实际结果输出；状态不能混淆“效果后端”和“虚拟摄像头 GPU 捕获/转换”。
3. 720p30 输出持续推进，P99 GPU→CPU 回读与整链耗时处于单帧预算内，队列丢帧有界且不反压本地播放；阈值由 Phase 0 基准固定后写入自动门禁。
4. 不存在 CPU 重做 GPU83、GDI/截图轮询、第二 mpv、逐帧视频编码或 Go 媒体转发。
5. Windows 10/11 默认使用 DirectShow，32 位和 64 位目标应用都能看到同名设备；Windows 11 MF 只有通过独立实验门禁后才能条件启用。
6. 停止、换源、窗口重建、D3D 设备丢失、应用退出和卸载均有界回收，虚拟摄像头故障不改变本地播放。
7. GPL-3.0、对应源码、修改说明、SBOM、固定提交/补丁和 Authenticode 门禁全部通过后，才能生成正式发布包。

## 10. 后续可选增强

当且仅当首版末端回读成为已测量的性能瓶颈，才设计 AkVirtualCamera D3D11 共享纹理协议：使用共享 NT handle、adapter LUID、格式/尺寸、generation、fence/keyed mutex 和超时回收，保留 CPU 原始帧协议作为兼容基线。该增强需要修改上游消费者与服务、重新完成 GPL/签名/DirectShow/MF/多进程权限矩阵；在完成前不得写入当前能力状态。
