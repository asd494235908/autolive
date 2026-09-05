# C# ↔ Rust 同步契约 fixture

这些 JSON 是跨语言行为合同，不是任一端的序列化产物。两端测试应读取相同的
`input`，验证 `expected` 中的状态、错误类别、资源所有权和禁止行为。

- `media-effects.json`：GPU83 → CPU4 → Original 单向降级。
- `playback-state.json`：单窗口 EOF 换源，不生成版本文件。
- `audio-candidate.json`：N/N+1 切换和最终 PCM 双消费者。
- `rtmp-output.json`：直接媒体读取、最终 PCM、有限重试。
- `virtual-camera.json`：AkVirtualCamera 固定输出规格和 GPU 边界。
- `douyin-m1.json`：`WebcastChatMessage`、本地回复池、有界串行发送。
- `error-codes.json`：两端 UI 可映射的稳定错误类别，不要求底层异常文本一致。

fixture 不包含真实地址、stream key、Cookie、Token、API Key、用户弹幕正文或本机路径。
