# Prototype Instructions

Run the local server yourself and open the preview in the browser available to this environment. Do not give the user server-start instructions when you can run it.

Before making substantial visual changes, use the Product Design plugin's `get-context` skill when the visual source is unclear or no longer matches the current goal. When the user gives durable prototype-specific design feedback, preferences, or decisions, record them in `AGENTS.md`.

When implementing from a selected generated mock, treat that image as the source of truth for layout, component anatomy, density, spacing, color, typography, visible content, and hierarchy.

Build app UI in `src/`. Keep `.openai/hosting.json`, `worker/index.js`, `scripts/prepare-sites-build.mjs`, and `tests/sites-worker.test.mjs` intact so the same local prototype can be handed to Sites. Before a Sites handoff, run `npm run build` and `npm run test:sites`; the build must leave `dist/client/index.html`, `dist/server/index.js`, and `dist/.openai/hosting.json`.

Durable homepage layout decision (2026-08-26): use an “音频” card above an independent “插话声音预设” card in the left media column and a “画面” card in the right column. Keep interruption cycle/status/current preset before ordinary audio inside “音频”, but do not render an extra visible “插话音频” subsection title or its explanatory subtitle; render the 35 current parameter values in the separate preset card below. Arrange the shared “实际出口 / 处理状态 / 当前预设” facts horizontally in three compact columns at normal desktop widths, keep each fact label and value on the same left-right row, and use a narrow-screen single-column fallback without changing that inner row direction. Put ordinary-audio and video-processing controls in their card title bars. In the “声音功能” card below the final-effect-window card, label the advanced-audio drawer shortcut “模式修改” and keep “随机插话” beside it. Do not delete or rename the underlying processing, drawer, IPC, or parameter functionality.
