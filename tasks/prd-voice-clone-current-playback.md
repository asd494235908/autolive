# PRD：当前文案克隆语音循环播放

## Document Status

- Status: Draft
- File Mode: Split
- Current Phase: Not Started
- Last Updated: 2026-08-14
- Context File: [context.md](./prd-voice-clone-current-playback/context.md)
- Purpose: 作为“导入视频自动准备人声、只播放当前文案、播放结束恢复原音轨”的唯一实施依据。

## Problem

当前固定话术流程是在播放中按当前位置生成局部替换音轨，容易出现模型重复加载、替换按钮状态不一致和循环时音轨恢复逻辑不符合新需求的问题。新流程需要把“提取参考人声”“生成当前文案”“临时静音原音轨”“克隆声音结束后恢复原音轨”拆成明确的状态和生命周期。

## Goals

- G-1：导入 MP4 后自动提取人声，使用现有 Demucs、Whisper 和 XTTS-v2 链路。
- G-2：10 条预制文案和手动输入仅作为可选文案，任意时刻只生成并播放当前文案。
- G-3：点击播放当前文案时记录视频位置，立即静音原音轨并播放克隆声音。
- G-4：当前克隆声音播放结束后恢复原音轨。
- G-5：视频循环不自动触发克隆文案；下一轮在用户没有再次点击时继续播放原音轨。
- G-6：相同视频、参考人声、文案和模型版本命中缓存，不重复提取或生成。
- G-7：不影响原有视频处理、普通声音处理、单窗口播放和其他音频能力。

## Non-Goals

- NG-1：不把 10 条预制文案组成播放队列，也不连续播放多条文案。
- NG-2：首版不把原视频所有 ASR 片段自动改写成完整新旁白；当前只处理当前选中的一条文案。
- NG-3：不生成 N 个 MP4，不创建媒体版本队列，不接入 NVIDIA 专用流程。
- NG-4：不通过服务器代理音频或模型调用；模型和音频处理仍在桌面端本地执行。
- NG-5：不删除现有“按当前位置替换当前话术”兼容能力，待新流程验证后再单独决定是否下线。

## Success Criteria

- SC-1：导入 MP4 后，固定话术区域自动进入“提取人声/准备模型”状态；成功后显示参考人声已准备。
- SC-2：选择或修改文案只生成当前文案，不能触发其他预制文案的批量生成或播放。
- SC-3：点击播放当前文案时，原音轨在克隆音频开始前已静音，播放期间无原声泄漏。
- SC-4：克隆音频 `ended` 后，原音轨恢复到视频当前播放位置并继续播放。
- SC-5：视频循环与克隆文案互不触发；用户没有再次点击时，下一轮只播放原音轨。
- SC-6：暂停、继续、拖动、停止、取消和更换视频后，不残留旧克隆音频、错误静音或旧异步结果。
- SC-7：同一缓存键不会重复调用 XTTS-v2；更换视频或当前文案后会生成新的缓存产物。
- SC-8：现有 Python、Rust、React 测试及本地构建通过，人工测试能覆盖至少两轮循环和文案结束恢复原声。

## Key Scenarios

### Scenario 1：导入视频并准备当前文案

- Actor：桌面端用户
- Trigger：选择 MP4 并导入
- Expected outcome：视频保持现有播放能力；后台自动提取人声和准备 XTTS-v2；当前文本准备完成后生成一条可复用克隆音频。

### Scenario 2：播放当前文案

- Actor：桌面端用户
- Trigger：点击“播放当前文案”
- Expected outcome：记录当前视频位置，静音原音轨，播放当前文案对应的克隆音频；不播放其他预制文案。

### Scenario 3：克隆音频结束

- Actor：播放器
- Trigger：克隆音频触发 `ended`
- Expected outcome：停止克隆音频，恢复原音轨，并从当前视频位置继续同步播放。

### Scenario 4：视频循环但用户没有再次点击

- Actor：播放器
- Trigger：视频结束并回到下一轮 0 秒，用户没有再次点击播放当前文案
- Expected outcome：不启动、不重播克隆音频；原音轨继续播放。若上一条克隆音频仍未结束，则保持原音轨静音直到该音频自然结束。

### Scenario 5：切换文案或视频

- Actor：桌面端用户
- Trigger：切换预制文案、修改手动文案或重新导入视频
- Expected outcome：旧文案异步任务和旧音频引用失效；新视频重新提取人声，新当前文案生成新缓存。

## Discovery Summary

- Reviewed：`产品需求文档.md`、`系统架构总览.md`、`桌面客户端架构.md`、`媒体参数范围与默认值.md`、`长任务开发总计划.md`、`实时音频幻化与循环播放方案.md`。
- Reviewed code：`desktop/worker/voice_clone_adapter.py`、`desktop/src-tauri/src/commands.rs`、`desktop/src-tauri/src/lib.rs`、`desktop/src-tauri/src/voice_clone.rs`、`desktop/ui/src/App.tsx`、`desktop/ui/src/voiceCloneStatus.ts`。
- Current system：导入后由 React 自动等待 MP4 哈希并调用 `prepare_voice_clone_source`；Worker 使用 Demucs 分离人声、Whisper 建立片段索引并加载 XTTS-v2；当前替换操作使用 `replace_current` 生成从当前位置到恢复位置的完整替换 WAV；React 已有隐藏的克隆音频槽和循环边界处理。
- Design implication：新需求只需要当前文案的短音频和明确的“克隆播放中/原音轨恢复”状态，不应继续把每次点击建模成整条源音轨局部拼接；循环边界不得重启或清理仍在播放的克隆片段；当前音频处理和实时音频候选的优先级必须保持不变。
- Validation surface：Python `unittest`、Rust `cargo fmt`/`clippy`/`cargo test`、React `pnpm test`/`pnpm build`、Tauri 开发版人工测试。
- Confidence / gaps：用户已确认克隆文案只由点击触发一次，与视频循环无关；用户未要求把一条文案映射到多个原话术片段，因此首版明确为单条当前文案。

## Requirements

### Functional Requirements

- FR-1：导入成功并完成源哈希后，自动启动当前视频的人声准备；同源缓存命中时不得重新执行分离。
- FR-2：当前文案来源只能是当前选中的预制文案或当前手动输入文本；生成请求不得携带文案队列。
- FR-3：文案生成输出必须是独立的、可校验的本地音频片段，不得覆盖源 MP4 或原始音轨文件。
- FR-4：开始克隆播放前必须把原视频音轨置为静音或停止输出；克隆音频未准备好时不得漏出原音。
- FR-5：克隆音频播放结束、取消、停止或失败时，按照状态机恢复原音轨或保持用户明确选择的静音状态。
- FR-6：每次点击只播放当前文案一次；视频循环不自动重播克隆文案，不自动切换预制文案，不合并多条文本。
- FR-7：源视频代际、源/参考/文案哈希和操作 ID必须参与异步结果校验，旧结果不得提交；正常循环轮次变化不得使当前克隆音频失效。
- FR-8：修改当前文案后，当前克隆缓存标记失效；正在播放的旧片段完成或被明确取消后，不得自动切换成其他文案。

### Non-Functional Requirements

- NFR-1：继续使用常驻 Worker 和进程内 XTTS-v2 模型缓存，播放期间不得重复下载或初始化模型。
- NFR-2：缓存产物通过临时文件、完整 SHA-256、FFprobe 元数据校验和原子提交保护。
- NFR-3：Worker、Rust 和 UI 均需提供 loading、failed、cancelled、stale 和恢复原声状态。
- NFR-4：不改变普通声音处理、实时音频候选、视频处理和单窗口循环的既有边界。
- NFR-5：所有新增异步任务都有取消、超时、资源清理和旧结果拒绝路径。

## Assumptions

- A-1：用户所说的“当前文案”是当前文本框内容；选择预制文案只是把文本加载到当前文本框。
- A-2：克隆文案是一次性点击触发；视频循环时不会自动重播当前文案，也不会自动播放其他 9 条预制文案。
- A-3：点击开始克隆播放时使用视频当前播放位置；视频循环只重置视频和原音轨时间轴，不重置或重播克隆音频。
- A-4：用户修改文案时只影响下一次克隆播放或下一轮，不在当前克隆片段中途切换，避免叠音和状态竞态。
- A-5：普通视频导入后的原有播放行为保留；只有克隆音频实际播放窗口内临时静音原音轨。

## Dependencies / Constraints

- 本地依赖：FFmpeg/FFprobe、Demucs、faster-whisper、Coqui TTS XTTS-v2；正式包必须包含完整模型资源或明确提示资源缺失。
- 既有 Worker 协议：继续使用常驻 `--serve` JSON 行协议，新增操作不能破坏 `prepare_source` 和兼容的 `replace_current`。
- 既有媒体状态：`playback_generation`、`loop_index`、当前音频来源和音频槽机制是事实源，React 不得自行复制播放状态机。
- 路径安全：所有 Worker 输出路径必须来自 Rust 创建的受控缓存目录，前端不能自由拼接路径。

## Risks / Edge Cases

- 模型首次加载时间较长：导入准备阶段显示进度，克隆播放未就绪时保持原音轨静音，不能播放错误音轨。
- 克隆音频比当前视频剩余时间更长：允许它跨越视频循环继续播放；在它自然结束前原音轨保持静音，结束后恢复原音轨。
- 克隆音频短于视频：音频 `ended` 后恢复原音轨，视频继续播放。
- 用户在克隆音频播放中暂停或拖动：先暂停/停止克隆音频，再按新位置恢复或等待下一次播放命令。
- 文案或源视频变更后旧 Worker 返回：按 generation/source/text hash 拒绝旧结果并清理临时文件。
- 原音轨本身处于普通声音处理或实时音频候选状态：新功能必须明确占用关系，不能同时输出两条音频。

## Phase Index

| Phase | Status | Objective | Validation Focus | File |
|---|---|---|---|---|
| Phase 1：状态、契约与缓存键 | Not Started | 建立单条当前文案的播放状态和缓存身份 | Rust 单元/集成测试 | [phase-01-状态与缓存.md](./prd-voice-clone-current-playback/phase-01-状态与缓存.md) |
| Phase 2：Worker 当前文案生成 | Not Started | 新增独立短音频生成操作并复用模型缓存 | Python 单元测试与 Worker 协议测试 | [phase-02-worker生成.md](./prd-voice-clone-current-playback/phase-02-worker生成.md) |
| Phase 3：Rust 调度与导入准备 | Not Started | 自动准备人声、命中缓存、取消和过期保护 | Rust 集成测试 | [phase-03-rust调度.md](./prd-voice-clone-current-playback/phase-03-rust调度.md) |
| Phase 4：React 播放与循环同步 | Not Started | 临时静音原音轨、播放当前文案、结束恢复、循环不触发 | UI 单元测试与人工播放器测试 | [phase-04-ui播放.md](./prd-voice-clone-current-playback/phase-04-ui播放.md) |
| Phase 5：完整验证与打包 | Not Started | 验证不影响既有功能并确认正式包模型资源 | 全量测试、构建、人工 smoke test | [phase-05-验证与打包.md](./prd-voice-clone-current-playback/phase-05-验证与打包.md) |

## Final Multi-Pass Review After All Phases

- [ ] 1. Requirements coverage review：每条 FR/NFR 和 SC 都已实现或明确延期。
- [ ] 2. Cross-phase integration review：Worker、Rust 状态、UI 音频槽和循环边界使用同一事实源。
- [ ] 3. Correctness review：成功、失败、取消、超时、过期、暂停、拖动、循环和更换视频路径完整。
- [ ] 4. Simplicity/refactor review：不重复实现已有 Worker、缓存和播放同步能力。
- [ ] 5. Duplication/cleanup review：删除新增未使用类型、导入、导出、配置、临时日志和废弃分支。
- [ ] 6. Security/privacy review：路径、模型资源、音频缓存和 IPC 输入受控。
- [ ] 7. Performance review：相同缓存键不重复提取/加载/生成，长视频循环不创建无界音频文件。
- [ ] 8. Validation review：自动化检查和人工测试覆盖克隆结束恢复原音轨、循环不触发和当前文案唯一性。
- [ ] 9. Documentation/operability review：开发版、正式打包版模型资源和错误提示一致。
- [ ] 10. PRD closeout review：状态、变更记录、剩余风险和后续工作已更新。

## Open Questions

- 当前没有阻塞性问题；“每次点击只播放当前文案一次，视频循环不自动触发”已按用户确认固化为 FR-6。

## Change Log

- 2026-08-14：根据用户确认创建当前文案克隆语音临时替换和循环播放 PRD。
