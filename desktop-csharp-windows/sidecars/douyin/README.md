# C# 抖音持续弹幕 sidecar

> 2026-09-10 登录诊断：新增固定白名单 `auth.diagnostic` 事件，记录取码、轮询状态变化、登录回跳、账号校验及失败类别。只采集安全的数字状态码和验证要求标记，不打印响应、异常原文、请求 URL 或任何凭据；C# 负责校验、有界落盘和日志位置展示。见 [登录脱敏日志记录](../../docs/2026-09-10-抖音登录脱敏日志实施记录.md)。

2026-09-09：代码已接入；用户已在真实 WPF 开发端扫码登录，小窗口已显示真实连续弹幕，关闭重开后消息保留。详细观察时长及未验收项见 [实施记录](../../docs/2026-09-09-抖音实时弹幕窗口实施方案.md)。

## 入口与来源

`sidecar.py --upstream-root <checkout>` 由 C# canonical 宿主通过 Conda 启动。
上游为 [cv-cat/DouYin_Spider](https://github.com/cv-cat/DouYin_Spider)，固定提交
`9afaf79580b1ee84e8954ff906ff26869d5b7f1f`。开发 checkout 在
`desktop-csharp-windows/.tools/douyin-upstream`，不放进提交、不修改上游、不使用 worktree。
启动时校验实际 HEAD 及跟踪文件未修改。该快照未找到独立 LICENSE 文件；本地兼容验证不等于已经满足再分发许可，安装包继续保留许可核验门禁。

`upstream_adapter.py` 复用上游二维码认证、房间解析、签名和 Protobuf，延续已有 Rust 探针的 WSS 参数。
不导入上游交互 CLI，不运行一次性探针，不自行实现平台协议或增加服务框架。

## 协议和所有权

stdin/stdout 为 `WindowsDouyinSidecarProtocol` NDJSON v1；单行最多 64 KiB，逐行消费，没有累计输出寿命。
处理 `auth.qr.start`、`auth.cancel`、`auth.logout`、`live.open`、`live.close`、`chat.send`、`shutdown`。
主线程处理命令，扫码和 WSS 分别使用一个受管线程。扫码默认 300 秒；HTTP 请求强制验证 TLS 且单次 10 秒；WSS 握手 10 秒、接收轮询 1 秒；已连接会话持续运行。
断开与退出设置取消、关闭 socket、Join；上游扫码等待使用可取消 Event，HTTP 返回边界检查取消。C# Job Object 负责进程级兜底。

`live.close` 只关闭当前房间和 Join WSS 线程，保留本次 Python 进程内存中的登录；再次 `live.open` 用同一认证对象创建新的 session/generation，不发起扫码。`auth.logout`、`shutdown`、stdin EOF 或软件退出清除内存登录；重启软件仍需扫码。明确 HTTP/WSS 401 分类为 `auth_expired` 并丢弃失效认证；未知网络失败不冒充登录失效。

成功 `live.open` 首先返回 session/generation 归属，再发 `live.state`、连续 `live.chat`；正常断开输出 `closed`、异常输出 `failed`，不假装仍连接，不自动重连。
昵称、正文、接收时间、消息 ID 和本人标记只传本机 IPC，不进入 stdout 调试日志或磁盘。
上游 Protobuf 没有独立历史消息时间字段；适配层将首次 REST 快照中的消息 ID 及最近 5000 个已见 ID 标记 replay，C# 保留自身去重和禁止回复 replay 的规则。

观看模式零发送；只有明确 `chat.send` 命令且当前会话已连接才调用上游发送，检查会话代际及 canonical 协议的 100 字符/400 字节上限；产品回复池的 80 字符/320 字节限制仍由 Core 控制。最近 5000 个 action ID 用于防重复，超出时淘汰最旧 ID，不因长会话累计发送达到 5000 次而停发。
平台成功响应只表示 `accepted`，本人回显仍由上层验收；网络发送结果不明返回 `unknown`，不会自行重发。
上游 `sendMsgInRoom` 调用 `/webcast/room/chat/` 并返回 JSON；只有整数 `status_code=0` 作为 accepted，明确非零为 rejected，无法识别的返回形状为 unknown。自回显来自持续 WSS 的 `WebcastChatMessage`，用当前登录 UID 标记 `is_self`；只有真实回显才证明看到消息。
没有回复池、自动发送定时器、数据库、模型调用或历史导出。

所有登录凭据仅内存；禁用上游 `.env` 加载并移除继承的 `DY_*` 环境配置。
QR 在内存转 PNG 后通过 `auth.qr` 输出，不生成图片文件。第三方 print/loguru 输出关闭；stderr 仅错误类别与函数/行号，不含异常正文、QR 链接、Cookie、平台消息正文或签名 URL。

`login_diagnostics.py` 只观察已有 `get_qrcode`、`check_qrcode`、登录回跳和账号校验调用；返回值、异常和重试均由原链路处理。HTTP 观察沿用既有受限请求包装，TLS 仍强制开启。诊断每次登录最多 64 项，去重并保留失败终态位置，避免轮询刷盘。`verification_required` 仅依据明确验证响应头的存在性，不能据普通失败推测，也不自动处理验证。`platform_code` 只来自 JSON 中有界整数 `error_code/status_code`，不解析异常正文。

## 本机验证

使用 `AUTOLIVE_CONDA_ENV` 指定环境，否则为 `gpautolive-douyin`，禁止系统 Python 或 `.venv`。
依赖沿用现有探针库，`requirements.lock.txt` 锁定本次实际验证的直接及传递运行库；无新增框架。

```powershell
& E:/Miniconda3/Scripts/conda.exe run -n gpautolive-douyin python -m pip install -r desktop-csharp-windows/sidecars/douyin/requirements.lock.txt
& E:/Miniconda3/Scripts/conda.exe run -n gpautolive-douyin --no-capture-output python -m unittest discover -s desktop-csharp-windows/sidecars/douyin -p test_sidecar.py -v
& E:/Miniconda3/Scripts/conda.exe run -n gpautolive-douyin --no-capture-output python desktop-csharp-windows/sidecars/douyin/check_qr_bootstrap.py
```

离线测试覆盖连续接收零发送、二维码取消和线程结束、明确发送与重复 action、旧代际拒绝、输入行上限、shutdown，以及真实上游 Protobuf 字段、ACK、本人标记和重放。
`check_qr_bootstrap.py` 为显式联网检查：最多等 60 秒获取 QR，立即 shutdown；只打印状态，不展示二维码，不扫码、不发送弹幕。
本次 QR bootstrap 成功取得真实二维码并正常退出。随后用户在 WPF 开发端扫码登录，已实际看到直播间连续弹幕；观看时自动回应关闭、回复队列保持 0。本次没有测试真实发送或自账号回显。

消融检查：没有新增通用接口、队列或自动重试框架；保留两个有界 ID 集、线程取消和 TLS 校验，用于防止重复发送、错误归属及退出残留。

## 单次发送与登录复用验收

`check_single_send.py` 只在用户明确授权发送时由操作者显式运行；启动必须带 `--send-once 大家好`，不允许其他正文。它扫码一次、连接并最多观察 60 秒公屏、关闭房间、同一进程直接重新连接，然后只发送一次固定问好，分别记录平台响应与最长 30 秒内的本人回显。没有公屏消息时报告 0 条，仍继续重连和单次发送；unknown、拒绝或超时绝不重试。

二维码必须写入操作者指定的全新 `.png` 路径；已有文件直接拒绝启动。仅更新本脚本创建的二维码，结束时删除该文件，凭据仍不落盘。控制台仅输出阶段、二维码文件路径、条数、响应状态与回显结果，不打印二维码 URL、Cookie 或公屏正文。

```powershell
& E:/Miniconda3/Scripts/conda.exe run -n gpautolive-douyin --no-capture-output python desktop-csharp-windows/sidecars/douyin/check_single_send.py --room-id 362781620214 --qr-output "$env:TEMP/gpautolive-single-send-unique.png" --send-once 大家好
```

此命令会执行一次真实发送，不属于离线自动化测试；实现时未自动运行。
