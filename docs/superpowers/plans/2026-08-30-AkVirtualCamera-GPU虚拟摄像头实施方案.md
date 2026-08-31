# AkVirtualCamera GPU 虚拟摄像头实施方案

> 状态：正式需求·待实施
> 更新日期：2026-08-30
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

- [ ] 锁定 AkVirtualCamera 提交，完成 GPL-3.0 分发/修改法律审核和对应源码方案。
- [ ] 本机构建并安装上游 DirectShow x86/x64；Windows 11 仅把 Media Foundation 作为实验门禁验证。禁止把源码放到服务器构建。
- [ ] 用上游 test pattern/CPU 原始帧仅验证设备注册、枚举、卸载和下游兼容性；该结果不代表产品 GPU 链完成。
- [ ] 建立 WGC HWND → D3D11 texture 技术样例，证明能捕获真实 mpv/libplacebo 最终效果。
- [ ] 完成 D3D11 GPU 转换 + 三槽异步回读基准。720p30 必须通过后才进入产品接线；不通过则先评估共享 D3D11 纹理扩展。

### Phase 1：最小 GPU 闭环

- [ ] 新增 `VirtualCameraOutputManager`、状态机、取消、超时、generation 和资源释放测试。
- [ ] 接入 WGC、D3D11 GPU 缩放/色彩转换和三槽 staging latest-wins。
- [ ] 接入 AkVirtualCamera 原始帧投递，固定 `720p30`，无客户端时不泄漏资源。
- [ ] 最终窗口重建、暂停、seek、循环、切源、纯音频和处理开关变化不创建第二播放器、不改变本地播放。

### Phase 2：产品 UI 与安装

- [ ] 添加最小输出卡、安装/修复、开关、规格、实际状态和错误；使用 Ant Design，不新增重复组件。
- [ ] NSIS 增加精确组件安装/卸载、x86/x64 注册、Windows 11 MF 条件注册、签名和回滚。
- [ ] 增加 GPL 文案、修改声明、SBOM、完整对应源码和固定哈希发布门禁。

### Phase 3：兼容性与性能

- [ ] 通过 Windows 10/11、Intel/NVIDIA/AMD、多显示器、100%/125%/150% DPI 和 D3D 设备重置矩阵。
- [ ] 下游覆盖 Chrome、Edge、Teams、Zoom、Discord、OBS、Windows Camera，以及至少一个 32 位 DirectShow 客户端。
- [ ] 720p30 连续 2 小时无资源增长；1080p30 只有在同等门禁通过后才开放。
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
