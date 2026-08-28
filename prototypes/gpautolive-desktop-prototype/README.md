# GpAutoLive 桌面端 HTML 原型

本目录是依据参考图与当前桌面端产品范围制作的独立交互原型。它用于评审信息结构、视觉密度与主要操作，不接入 Tauri IPC、FFmpeg、PortAudio、账号服务或真实文件处理。

最新插话当前值验收截图：[`artifacts/interruption-audio-current-values.png`](./artifacts/interruption-audio-current-values.png)。
最新左右布局与紧凑参数验收截图：[`artifacts/layout-audio-left-compact-rows.png`](./artifacts/layout-audio-left-compact-rows.png)。
最新抽屉统一验收截图：[`artifacts/audit-home-gap/03-advanced-audio-unified.png`](./artifacts/audit-home-gap/03-advanced-audio-unified.png)、[`artifacts/audit-home-gap/04-fixed-speech-unified.png`](./artifacts/audit-home-gap/04-fixed-speech-unified.png)。
最新主页验收截图：[`artifacts/audit-home-gap/06-prototype-home-final.png`](./artifacts/audit-home-gap/06-prototype-home-final.png)。
最新播放池与 PortAudio 验收截图：[`artifacts/playback-pool-inline-and-portaudio-settings.png`](./artifacts/playback-pool-inline-and-portaudio-settings.png)。
最新独立播放控制与单一主页导航截图：[`artifacts/playback-nav-update/04-home-nav-only.png`](./artifacts/playback-nav-update/04-home-nav-only.png)。
最新异常与恢复路径截图：[`artifacts/runtime-recovery/02-engine-error.png`](./artifacts/runtime-recovery/02-engine-error.png)、[`artifacts/runtime-recovery/03-engine-recovering.png`](./artifacts/runtime-recovery/03-engine-recovering.png)、[`artifacts/runtime-recovery/05-interruption-error-focused.png`](./artifacts/runtime-recovery/05-interruption-error-focused.png)。
桌面端主页功能差异：[`desktop-home-gap-audit.md`](./desktop-home-gap-audit.md)。

## 原型范围

- 最多 100 项的视频与纯音频统一本地有序播放池及单窗口顺序循环；整批替换、追加、拖放追加、单项替换、拖拽/键盘排序、删除和清空全部直接位于主页播放池 Card，不再打开管理抽屉，失败保持旧池
- 播放、暂停、继续、停止分别使用独立按钮；每个动作有自己的 loading，动作执行期间禁用其他播放动作，另保留进度、循环次数、音量与画中画入口
- 顶部工作区导航只保留主页，不展示未实现独立页面的“设置”和“状态”入口
- 媒体引擎不可用、视频处理失败、声音处理失败与随机插话失败均在对应功能 Card 内条件式展示；错误包含原因、恢复操作和恢复中的 loading，成功恢复后自动移除，健康状态不渲染占位。右栏“实时处理引擎/本地运行状态”只同步聚合状态
- 相互独立的视频处理和普通声音处理开关
- 普通声音周期默认范围为 `3–5 秒`，视频周期为 `8–15 秒`；两组可编辑输入分别位于“音频”和“画面”Card 的周期状态块内，左侧播放列不再重复展示周期范围。随机插话周期保持只读并由插话配置控制，其变化周期在抽屉中统一使用秒输入，内部模拟状态继续以毫秒保存
- 完整展示当前正式模型的 111 项参数（普通视频 32、普通声音 37、高级视觉 42），使用 28px 紧凑单行纯文字参数框；参数名称 10px、参数值 11px
- 中间主体右列为“画面”Card，左列依次为“音频”和独立“插话声音预设”Card；音频 Card 内直接展示插话声音周期、出口、状态、当前预设和最终混音波形，再展示普通声音，不显示额外的插话分区标题和说明。其中“实际出口 / 处理状态 / 当前预设”在宽屏横向紧凑排列，并且每项名称和值保持左右同一行；窄屏自动回落为单列。35 字段当前单值集中在下方独立预设 Card。普通声音处理与视频处理开关位于各自 Card 标题栏。右侧“声音功能”Card 位于最终效果窗口下方，承载“模式修改”与随机插话两个既有抽屉入口；视频周期和底层参数语义保持不变
- 参数行压缩为 28px 高度、网格最小宽度收紧为 126px；固定视觉频段权重按正式 `band_weights.<Hz>` 路径拆为 12 个独立参数行，Rust 事实字段仍是 `AdvancedEffectParams.band_weights`
- 仅对三张参考截图对应的声音/视频/插话周期卡及普通声音/插话状态卡采用紧凑密度；不改变参数行、外层 Card、播放池、PortAudio 或其他模块
- 高级声音、随机插话和固定话术抽屉统一使用随机插话确立的双行标题、状态摘要、Card 分区、独立滚动主体与固定底部操作；高级声音只保留独立“应用声音参数”，不再承载 PortAudio 设备配置
- 右栏保留最终效果窗口、PortAudio 设备配置、固定话术入口、本地处理链与本地运行状态；PortAudio 位于话术功能上方，可选择 Host API 与输出设备、设置 `128–2048 KiB` 内存缓冲并显式应用，同时持续展示实际出口、已应用配置和处理状态，并保留与桌面端一致的固定 `440 Hz / 400 ms` 本地测试音入口；不展示声音与视频周期联动入口

普通声音全部参数继续只读展示，不设置“四项可编辑基线”。固定话术最多 10 条预制的增删改、停止朗读和独立状态属于下个版本，本原型不实现。

本版本不展示实时话术幻化、speech-to-speech、模型租约、RTMP/OBS、生成版本队列、研究报告或检测规避指标。

异常状态可通过独立评审地址查看：`?issue=engine`、`?issue=video`、`?issue=audio`、`?issue=interruption`；正常地址仍为无错误的健康状态。这些查询参数只用于 HTML 原型展示，不是正式桌面端接口。

## 本地命令

```powershell
npm run dev -- --port 4173 --strictPort
npm test
npm run build
npm run test:sites
```

视觉回归证据见 [`design-qa.md`](./design-qa.md)。插话独立卡截图位于 [`artifacts/interruption-audio-card.png`](./artifacts/interruption-audio-card.png)，聚焦对照图位于 [`artifacts/interruption-audio-comparison.png`](./artifacts/interruption-audio-comparison.png)。
