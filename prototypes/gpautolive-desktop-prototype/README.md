# GpAutoLive 桌面端 HTML 原型

本目录是依据参考图与当前桌面端产品范围制作的独立交互原型。它用于评审信息结构、视觉密度与主要操作，不接入 Tauri IPC、FFmpeg、PortAudio、账号服务或真实文件处理。

## 原型范围

- 最多 100 项的本地有序播放池与单窗口顺序循环
- 播放、暂停、继续、停止、进度、音量与画中画入口
- 相互独立的视频处理和普通声音处理开关
- 普通视频、高级视觉、普通声音参数的只读状态展示
- 随机插话、固定话术和高级声音设置抽屉
- 最终效果窗口、音频诊断与本地处理链状态

本版本不展示实时话术幻化、speech-to-speech、模型租约、RTMP/OBS、生成版本队列、研究报告或检测规避指标。

## 本地命令

```powershell
npm run dev -- --port 4173 --strictPort
npm test
npm run build
npm run test:sites
```

视觉回归证据见 [`design-qa.md`](./design-qa.md)，最终桌面截图位于 [`artifacts/prototype-desktop-1728-final.png`](./artifacts/prototype-desktop-1728-final.png)。
