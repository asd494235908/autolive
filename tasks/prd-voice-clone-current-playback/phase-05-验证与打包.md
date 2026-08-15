# Phase 5：完整验证与打包

Parent PRD: [当前文案克隆语音循环播放](../prd-voice-clone-current-playback.md)

Status: Not Started
Last Updated: 2026-08-14

## Objective

完成全量自动化检查、桌面开发版人工测试和正式安装包资源检查，确认新流程只影响固定话术模块，不破坏已有桌面能力。

## Context From Master PRD

- Goals covered: G-1 至 G-7
- Success Criteria: SC-1 至 SC-8
- Requirements covered: 全部 FR/NFR

## Phase Discovery Gate

- [ ] 查看前四阶段实际 diff，确认没有未使用代码、重复缓存、残留旧按钮逻辑或未关闭后台任务。
- [ ] 检查 `git diff --check`、Rust/React/Python 测试入口和当前开发版启动脚本。
- [ ] 核对 `desktop/ui/package.json` 中 `tauri:build` 是否会先准备 FFmpeg、Worker 和语音模型资源。
- [ ] 确认本地测试素材是用户授权的 MP4，且至少包含人声和两轮可循环时长。

## Scope

### In Scope

- 自动化测试、格式化、Lint、类型检查和构建。
- 开发版人工 smoke test。
- 安装包模型资源和 Worker 资源检查。
- 文档、错误提示和剩余风险收口。

### Out of Scope

- 不进行服务器构建。
- 不引入 NVIDIA 支持。
- 不在本阶段扩展完整脚本旁白或多文案队列。

## Implementation Checklist

- [ ] 执行 Python 全量 Worker 测试并记录结果。
- [ ] 执行 Rust fmt、clippy 和全量测试，检查新增状态没有破坏现有音频候选和循环测试。
- [ ] 执行 UI `pnpm test` 和 `pnpm build`，检查没有 TypeScript 未使用导入、Vite 构建错误或测试快照遗漏。
- [ ] 执行 `pnpm voice-models:test` 和 `pnpm voice-worker:test`，确认模型、Worker 资源清单覆盖新命令依赖。
- [ ] 本地启动 `pnpm tauri:dev`，导入授权 MP4，等待自动提取人声和当前文案生成完成。
- [ ] 人工执行：播放当前文案、确认原音轨静音、等待文案结束、确认原声恢复；观察视频进入后续轮次时没有再次点击则始终播放原音轨。
- [ ] 人工执行：切换另一条预制文案和手动文案，确认只生成/播放最新当前文案；更换 MP4 后确认重新提取人声。
- [ ] 人工执行：暂停、继续、拖动、停止、取消、Worker 失败和模型资源缺失，确认错误提示和原音轨恢复行为。
- [ ] 执行 `pnpm tauri:build`，检查正式产物包含 Worker、FFmpeg/FFprobe 和 XTTS-v2 资源，且首次播放不会要求用户手工安装依赖。
- [ ] 执行 `git diff --check`，删除本次改动暴露的未使用代码、临时文件和调试日志；不处理与本功能无关的用户已有改动。

## Validation Strategy

此阶段采用分层验证：自动化检查证明契约和状态逻辑，Tauri 开发版证明媒体事件行为，打包检查证明正式资源闭环。任何未能执行的检查都要记录原因和替代证据。

## Validation Checklist

- [ ] `python -m unittest discover -s desktop/worker -p 'test*.py'`
- [ ] `cd desktop/src-tauri && cargo fmt --all -- --check`
- [ ] `cd desktop/src-tauri && cargo clippy --all-targets -- -D warnings`
- [ ] `cd desktop/src-tauri && cargo test --all-targets`
- [ ] `cd desktop/ui && pnpm test`
- [ ] `cd desktop/ui && pnpm build`
- [ ] `cd desktop/ui && pnpm voice-models:test`
- [ ] `cd desktop/ui && pnpm voice-worker:test`
- [ ] `cd desktop/ui && pnpm tauri:build`
- [ ] `git diff --check`
- [ ] 完成两轮视频循环人工 smoke test并记录未验证项。

## Exit Criteria

- [ ] 全量自动化检查通过，或每个失败项有明确原因和后续动作。
- [ ] 人工确认每次点击只播放当前文案一次，克隆结束恢复原声，视频循环不会自动触发文案。
- [ ] 正式安装包资源可用，不要求用户执行隐含命令下载模型。
- [ ] 其他视频/声音功能未出现回归。
- [ ] 主 PRD 更新为 Complete 或记录明确阻塞项。

## Phase-End Multi-Pass Review

- [ ] 1. 逐条核对 Success Criteria 和验收记录。
- [ ] 2. 核对开发版与正式包的资源路径和行为一致。
- [ ] 3. 检查所有异常路径均不会遗留静音或后台 Worker。
- [ ] 4. 评估是否有可以删除的兼容分支、重复缓存或重复状态。
- [ ] 5. 删除无用代码、导入、类型、测试夹具和日志。
- [ ] 6. 确认本地音频和模型路径没有越界访问。
- [ ] 7. 检查模型加载和缓存命中对长时间循环的性能影响。
- [ ] 8. 检查自动化和人工证据是否足以支撑交付结论。
- [ ] 9. 更新产品/架构文档中与旧“整轨局部替换”不一致的描述。
- [ ] 10. 更新主 PRD 状态、变更记录、剩余风险和后续事项。
