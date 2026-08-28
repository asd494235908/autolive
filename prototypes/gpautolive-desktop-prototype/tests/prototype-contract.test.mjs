import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const sourceFiles = [
  "src/App.jsx",
  "src/components/PlaybackColumn.jsx",
  "src/components/ParameterWorkspace.jsx",
  "src/components/AudioProcessingPanel.jsx",
  "src/components/InlineRecoveryAlert.jsx",
  "src/components/OutputColumn.jsx",
  "src/components/FeatureDrawers.jsx",
  "src/components/FixedSpeechDrawer.jsx",
  "src/components/FeatureDrawerShell.jsx",
];

test("prototype keeps the current desktop scope visible", async () => {
  const source = (await Promise.all(sourceFiles.map((file) => readFile(file, "utf8")))).join("\n");
  for (const required of ["播放池", "视频处理", "普通声音", "最终效果窗口", "随机插话", "固定话术"]) {
    assert.match(source, new RegExp(required), `missing required desktop capability: ${required}`);
  }
  for (const forbidden of ["实时话术幻化", "speech-to-speech", "RTMP", "OBS", "检测规避"]) {
    assert.doesNotMatch(source, new RegExp(forbidden), `out-of-scope capability leaked into UI: ${forbidden}`);
  }
});

test("desktop layout preserves three-column baseline and responsive stacking", async () => {
  const css = await readFile("src/styles.css", "utf8");
  assert.match(css, /320px\s+minmax\(0,\s*1fr\)\s+272px/);
  assert.match(css, /\.app-shell\s*\{[^}]*width:\s*100%/s);
  assert.doesNotMatch(css, /\.app-shell\s*\{[^}]*width:\s*100vw/s);
  assert.match(css, /@media\s*\(max-width:\s*1199px\)/);
});

test("parameter cards present values as text without decorative progress bars", async () => {
  const workspace = await readFile("src/components/ParameterWorkspace.jsx", "utf8");
  assert.match(workspace, /parameter-card__value/);
  assert.doesNotMatch(workspace, /parameter-card__progress/);
  assert.doesNotMatch(workspace, /parameter\.progress/);
});

test("parameter names and values share one row without connection badges", async () => {
  const [workspace, data, css] = await Promise.all([
    readFile("src/components/ParameterWorkspace.jsx", "utf8"),
    readFile("src/data/prototype-data.js", "utf8"),
    readFile("src/styles.css", "utf8"),
  ]);
  assert.match(workspace, /parameter-card__summary[\s\S]*parameter-card__label[\s\S]*parameter-card__value/);
  assert.doesNotMatch(workspace, /ParameterStatusTag|parameter-status-tag|参数接入状态说明/);
  assert.doesNotMatch(data, /PARAMETER_STATUS|status:\s*'implemented'/);
  assert.doesNotMatch(css, /parameter-status-tag/);
});

test("playback facts live in playback controls while media cards keep three requested cycles", async () => {
  const [app, playback, workspace, data] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/PlaybackColumn.jsx", "utf8"),
    readFile("src/components/ParameterWorkspace.jsx", "utf8"),
    readFile("src/data/prototype-data.js", "utf8"),
  ]);
  assert.match(playback, /播放进度/);
  assert.match(playback, /循环次数/);
  assert.match(app, /cycleCount=\{poolCycle\}/);
  assert.doesNotMatch(workspace, /STATUS_CARD_TEMPLATES|workspace-status-card|当前播放与处理状态/);
  assert.doesNotMatch(data, /STATUS_CARD_TEMPLATES|DEFAULT_PLAYBACK_POOL/);
  assert.equal((workspace.match(/<CycleStatusCard/g) ?? []).length, 3);
});

test("playback exposes independent play pause resume actions with isolated loading states", async () => {
  const [app, playback] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/PlaybackColumn.jsx", "utf8"),
  ]);

  for (const action of ["play", "pause", "resume"]) {
    assert.match(playback, new RegExp(`onPlaybackAction\\?\\.\\(\"${action}\"\\)`), `missing independent ${action} action`);
    assert.match(playback, new RegExp(`loading=\\{playbackActionBusy === \"${action}\"\\}`), `missing ${action} loading state`);
  }
  assert.match(app, /playbackActionBusy/);
  assert.match(app, /playbackActionBusy=\{playbackActionBusy\}/);
  assert.doesNotMatch(playback, /getPrimaryAction|primaryAction/);
});

test("desktop navigation only exposes the home entry", async () => {
  const [app, shell] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/AppShell.jsx", "utf8"),
  ]);

  assert.match(shell, />主页</);
  assert.doesNotMatch(shell, /label:\s*"设置"|label:\s*"状态"|>设置<|>状态</);
  assert.doesNotMatch(app, /activeView|setActiveView|onViewChange/);
});

test("ordinary sound cycle defaults to three through five seconds", async () => {
  const [app, workspace, data] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/ParameterWorkspace.jsx", "utf8"),
    readFile("src/data/prototype-data.js", "utf8"),
  ]);

  assert.match(app, /audioMin:\s*3,\s*audioMax:\s*5/);
  assert.match(workspace, /audioMin:\s*3,\s*audioMax:\s*5/);
  assert.match(data, /audio:\s*\{[\s\S]*?range:\s*['"]3–5 秒['"]/);
  assert.match(data, /random_change_period_ms[\s\S]*?value:\s*['"]4\.0 秒['"]/);
});

test("playback pool management supports the complete local editing workflow", async () => {
  const [app, playback] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/PlaybackColumn.jsx", "utf8"),
  ]);
  const playbackUsage = app.match(/<PlaybackColumn[\s\S]*?\/>/)?.[0] ?? "";
  const playbackSection = playback.match(/<section[\s\S]*?playback-pool-section[\s\S]*?<\/section>/)?.[0] ?? "";

  for (const label of ["追加媒体", "拖放", "替换", "上移", "下移", "删除", "清空"]) {
    assert.match(playback, new RegExp(label), `playback pool management missing: ${label}`);
  }
  assert.match(playback, /onDragOver=/);
  assert.match(playback, /onDrop=/);
  assert.match(playback, /dataTransfer\.files/);
  assert.match(playback, /dataTransfer\?\.types/);
  assert.match(playback, /includes\("Files"\)/);
  assert.match(playback, /if \(!draggedItemId\) return/);
  for (const callback of ["onPoolAppend", "onPoolReplaceItem", "onPoolMove", "onPoolRemove", "onPoolClear"]) {
    assert.match(playback, new RegExp(`\\b${callback}\\b`), `PlaybackColumn callback missing: ${callback}`);
    assert.match(playbackUsage, new RegExp(`${callback}=`), `App does not connect playback callback: ${callback}`);
  }
  assert.match(playbackSection, /onPoolMove|onPoolRemove|onPoolReplaceItem/);
  assert.match(playback, /\.mp3,.wav,.m4a,.aac,.ogg,.flac/);
  assert.doesNotMatch(playback, /poolDrawerOpen|playback-pool-drawer|播放池管理|<FeatureDrawerShell/);
  assert.doesNotMatch(playback, /只读|不追加|不重排|不删除/);
});

test("center keeps audio above an independent interruption preset card, beside one visual card", async () => {
  const workspace = await readFile("src/components/ParameterWorkspace.jsx", "utf8");
  const videoCardIndex = workspace.indexOf('media-domain-card--video');
  const audioCardIndex = workspace.indexOf('media-domain-card--audio');
  const presetCardIndex = workspace.indexOf('media-domain-card--interruption-preset');

  assert.equal((workspace.match(/className="media-domain-card media-domain-card--/g) ?? []).length, 3);
  assert.match(workspace, /media-domain-card--video[\s\S]*VIDEO_PARAMETERS[\s\S]*ADVANCED_PARAMETERS/);
  assert.match(workspace, /media-domain-card--audio[\s\S]*插话声音周期[\s\S]*AUDIO_PARAMETERS/);
  assert.match(workspace, /media-domain-card--interruption-preset[\s\S]*插话声音预设[\s\S]*presetChanges\.parameters/);
  assert.ok(videoCardIndex >= 0, "missing video domain card");
  assert.ok(audioCardIndex >= 0, "missing ordinary audio domain card");
  assert.ok(presetCardIndex > audioCardIndex && presetCardIndex < videoCardIndex, "interruption preset card must follow audio in the left column");
  assert.ok(audioCardIndex < videoCardIndex, "sound column must be on the left and video column on the right");
  assert.doesNotMatch(workspace, /parameter-workspace__cycles|parameter-workspace__parameter-columns/);
});

test("video actions stay inside the video card without a cross-media apply action", async () => {
  const [app, workspace] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/ParameterWorkspace.jsx", "utf8"),
  ]);
  const videoCard = workspace.match(/media-domain-card--video[\s\S]*$/)?.[0] ?? "";
  const audioCard = workspace.match(/media-domain-card--audio[\s\S]*?(?=media-domain-card--interruption-preset)/)?.[0] ?? "";

  assert.match(videoCard, />画面</);
  assert.match(videoCard, /media-domain-card__heading[\s\S]*视频处理[\s\S]*视频处理开关/);
  assert.match(videoCard, /恢复视频参数默认值[\s\S]*恢复默认/);
  assert.doesNotMatch(audioCard, /视频处理开关|恢复视频参数默认值/);
  assert.doesNotMatch(workspace, /音视频同时应用|应用视频|onApply/);
  assert.doesNotMatch(app, /onApply=/);
});

test("audio processing and requested audio features live inside the audio card", async () => {
  const [app, workspace, audioPanel, output, css] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/ParameterWorkspace.jsx", "utf8"),
    readFile("src/components/AudioProcessingPanel.jsx", "utf8"),
    readFile("src/components/OutputColumn.jsx", "utf8"),
    readFile("src/styles.css", "utf8"),
  ]);
  const audioCard = workspace.match(/media-domain-card--audio[\s\S]*?(?=media-domain-card--interruption-preset)/)?.[0] ?? "";
  const audioPanelUsages = workspace.match(/<AudioProcessingPanel[\s\S]*?\/>/g) ?? [];
  const parameterUsage = app.match(/<ParameterWorkspace[\s\S]*?\/>/)?.[0] ?? "";
  const outputUsage = app.match(/<OutputColumn[\s\S]*?\/>/)?.[0] ?? "";

  assert.doesNotMatch(workspace, /音视频处理 · 实时参数|parameter-workspace__header/);
  assert.match(audioCard, /AudioProcessingPanel/);
  assert.match(audioCard, /media-domain-card__heading[\s\S]*普通声音处理[\s\S]*普通声音处理开关/);
  assert.doesNotMatch(audioCard, /media-domain-card__actions/);
  assert.doesNotMatch(audioCard, /高级设置|advancedAudio|随机插话/);
  assert.match(audioCard, /interruption-audio-section[\s\S]*插话声音周期[\s\S]*实时 · 插话混音后 PCM/);
  assert.ok(audioCard.indexOf("media-domain-card__title-actions") < audioCard.indexOf("interruption-audio-section"));
  assert.match(audioPanelUsages[1] ?? "", /enabled=\{audioEnabled\}/);
  assert.doesNotMatch(audioPanelUsages[1] ?? "", /onEnabledChange|onOpenDrawer/);
  assert.match(audioPanel, /实时 · 混音后 PCM/);
  assert.doesNotMatch(audioPanel, /普通声音处理开关|高级设置|advancedAudio|随机插话|interruption|diagnostic-grid|DIAGNOSTICS|采样率|RMS/);
  assert.doesNotMatch(css, /diagnostic-grid|diagnostic-item|audio-processing-panel__heading|audio-processing-panel__features/);
  assert.doesNotMatch(output, /普通声音处理|普通声音处理开关/);
  assert.match(output, /声音功能[\s\S]*模式修改[\s\S]*随机插话/);
  assert.doesNotMatch(output, />\s*高级设置\s*</);
  assert.match(output, /fixedSpeech/);
  assert.match(output, /固定话术/);
  assert.match(parameterUsage, /onToggleAudio=\{setAudioEnabled\}/);
  assert.doesNotMatch(parameterUsage, /onOpenDrawer=/);
  assert.match(outputUsage, /onOpenDrawer=\{setActiveDrawer\}/);
  assert.doesNotMatch(outputUsage, /audioProcessingEnabled=|outputDevice=|audioStatus=|currentPreset=|waveform=|diagnostics=|onAudioProcessingChange=/);
});

test("interruption audio appears before the ordinary audio controls and cycle", async () => {
  const workspace = await readFile("src/components/ParameterWorkspace.jsx", "utf8");
  const audioCard = workspace.match(/media-domain-card--audio[\s\S]*?(?=media-domain-card--interruption-preset)/)?.[0] ?? "";
  const interruptionIndex = audioCard.indexOf('className="interruption-audio-section"');
  const ordinaryControlIndex = audioCard.indexOf('aria-label="普通声音处理开关"');
  const cycleIndex = audioCard.indexOf('title="声音周期"');
  const ordinaryProcessingIndex = audioCard.lastIndexOf("<AudioProcessingPanel");

  assert.ok(interruptionIndex >= 0, "missing interruption audio section");
  assert.ok(ordinaryControlIndex >= 0, "missing audio processing control in the card title");
  assert.ok(interruptionIndex > ordinaryControlIndex, "card-title audio control must remain visible above the interruption content");
  assert.ok(cycleIndex > interruptionIndex, "ordinary audio cycle must follow interruption audio");
  assert.ok(ordinaryProcessingIndex > cycleIndex, "audio cycle must precede the ordinary audio status panel");
});

test("audio card embeds interruption cycle, live status, and preset parameter changes", async () => {
  const [app, workspace, audioPanel, presetChanges, prototypeData] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/ParameterWorkspace.jsx", "utf8"),
    readFile("src/components/AudioProcessingPanel.jsx", "utf8"),
    readFile("src/data/interruption-preset-changes.js", "utf8"),
    readFile("src/data/prototype-data.js", "utf8"),
  ]);
  const parameterUsage = app.match(/<ParameterWorkspace[\s\S]*?\/>/)?.[0] ?? "";
  const audioCard = workspace.match(/media-domain-card--audio[\s\S]*?(?=media-domain-card--interruption-preset)/)?.[0] ?? "";
  const presetCard = workspace.match(/media-domain-card--interruption-preset[\s\S]*?(?=media-domain-column--video)/)?.[0] ?? "";
  const visibleCopy = `${audioCard}\n${audioPanel}`;

  assert.doesNotMatch(audioCard, />插话音频</);
  assert.doesNotMatch(audioCard, /独立声音周期与预设参数变化/);
  assert.match(audioCard, /title="插话声音周期"/);
  assert.doesNotMatch(audioCard, /<ParameterSection[\s\S]*title="插话声音预设"/);
  assert.match(presetCard, /插话声音预设/);
  for (const label of ["实际出口", "处理状态", "当前预设", "实时 · 插话混音后 PCM"]) {
    assert.match(visibleCopy, new RegExp(label), `interruption audio status missing: ${label}`);
  }
  assert.match(presetCard, /presetChanges\.parameters/);
  assert.match(workspace, /getInterruptionPresetChanges\(interruptionRuntime\)/);
  assert.match(presetChanges, /audio-value-presets\.ts/);
  assert.match(presetChanges, /AUDIO_VALUE_PRESETS/);
  assert.match(presetChanges, /AUDIO_PRESET_FIELDS/);
  assert.match(app, /DEFAULT_INTERRUPTION_RUNTIME[\s\S]*currentAudioPresetId:\s*["']p\d{2}["']/);
  assert.match(app, /interruptionRuntime=\{interruptionRuntime\}/);
  assert.match(presetChanges, /interruptionRuntime\.currentAudioPresetId/);
  assert.match(presetChanges, /AUDIO_VALUE_PRESETS\.find\([\s\S]*currentAudioPresetId/);
  assert.doesNotMatch(presetChanges, /formatRange|Math\.min|Math\.max|候选池|候选变化范围|～|–/);
  assert.doesNotMatch(presetChanges, /audioPresetIds|audioMixPickMin|audioMixPickMax/);
  for (const label of ["自然真人模式", "高质量音高", "输入增益", "高频扰动强度"]) {
    assert.match(`${presetChanges}\n${prototypeData}`, new RegExp(label), `interruption preset change mapping missing: ${label}`);
  }
  assert.match(workspace, /interruption\s*=/);
  assert.match(workspace, /interruptionRuntime\s*=/);
  assert.match(parameterUsage, /interruption=\{interruption\}/);
});

test("local runtime status lives only in the output column without a linked cycle control", async () => {
  const [app, playback, workspace, output, css] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/PlaybackColumn.jsx", "utf8"),
    readFile("src/components/ParameterWorkspace.jsx", "utf8"),
    readFile("src/components/OutputColumn.jsx", "utf8"),
    readFile("src/styles.css", "utf8"),
  ]);
  const playbackUsage = app.match(/<PlaybackColumn[\s\S]*?\/>/)?.[0] ?? "";
  const parameterUsage = app.match(/<ParameterWorkspace[\s\S]*?\/>/)?.[0] ?? "";
  const outputUsage = app.match(/<OutputColumn[\s\S]*?\/>/)?.[0] ?? "";

  assert.doesNotMatch(app, /linkedCycles|setLinkedCycles|onLinkedCyclesChange/);
  assert.doesNotMatch(playback, /本地运行状态|localStatus|声音与视频联动周期|linkedCycles|onLinkedCyclesChange|CycleRange|cycleRange|onCycleRangeChange/);
  assert.equal((workspace.match(/<CycleRange/g) ?? []).length, 1, "cycle cards share one range-input renderer");
  assert.equal((workspace.match(/onRangeChange=\{\(edge, value\) => onCycleRangeChange/g) ?? []).length, 2);
  assert.match(output, /本地运行状态[\s\S]*local-status-list[\s\S]*localStatus\.map/);
  assert.doesNotMatch(playbackUsage, /localStatus=/);
  assert.doesNotMatch(playbackUsage, /cycleRange=|onCycleRangeChange=/);
  assert.match(parameterUsage, /cycleRange=\{cycleRange\}/);
  assert.match(parameterUsage, /onCycleRangeChange=\{\(field, value\) => changeSetting\(setCycleRange, field, value\)\}/);
  assert.match(outputUsage, /localStatus=\{\[/);
  assert.doesNotMatch(css, /switch-setting-row/);
});

test("video and sound cycle inputs live in their media cards while interruption stays read-only", async () => {
  const workspace = await readFile("src/components/ParameterWorkspace.jsx", "utf8");
  const audioCard = workspace.match(/media-domain-card--audio[\s\S]*?(?=media-domain-card--interruption-preset)/)?.[0] ?? "";
  const videoCard = workspace.match(/media-domain-card--video[\s\S]*$/)?.[0] ?? "";
  const interruptionSection = audioCard.match(/interruption-audio-section[\s\S]*?(?=<InlineRecoveryAlert\s+issue=\{runtimeIssues\.engine\})/)?.[0] ?? "";

  assert.match(audioCard, /title="声音周期"[\s\S]*minValue=\{cycleRange\.audioMin\}[\s\S]*maxValue=\{cycleRange\.audioMax\}[\s\S]*onRangeChange=/);
  assert.match(videoCard, /title="视频周期"[\s\S]*minValue=\{cycleRange\.videoMin\}[\s\S]*maxValue=\{cycleRange\.videoMax\}[\s\S]*onRangeChange=/);
  assert.match(workspace, /aria-label=\{`\$\{title\}最小秒数`\}/);
  assert.match(workspace, /aria-label=\{`\$\{title\}最大秒数`\}/);
  assert.match(interruptionSection, /title="插话声音周期"/);
  assert.doesNotMatch(interruptionSection, /minValue=|maxValue=|onRangeChange=/);
});

test("runtime failures render beside their module with recovery actions and no healthy placeholder", async () => {
  const [app, workspace, recoveryAlert] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/ParameterWorkspace.jsx", "utf8"),
    readFile("src/components/InlineRecoveryAlert.jsx", "utf8"),
  ]);

  for (const issue of ["engine", "video", "audio", "interruption"]) {
    assert.match(app, new RegExp(`${issue}:\\s*\\{[\\s\\S]*?title:`), `missing ${issue} demo issue`);
  }
  for (const label of ["媒体引擎不可用", "视频处理失败", "声音处理失败", "随机插话失败"]) {
    assert.match(app, new RegExp(label), `missing failure copy: ${label}`);
  }
  assert.match(app, /new URLSearchParams\(window\.location\.search\)/);
  assert.match(app, /issueKey === "all"/);
  assert.match(app, /async function handleRuntimeRecovery\(issueKey\)/);
  assert.match(app, /setRuntimeIssues/);
  assert.match(app, /recoveryBusy/);
  assert.match(workspace, /runtimeIssues\.engine/);
  assert.match(workspace, /runtimeIssues\.video/);
  assert.match(workspace, /runtimeIssues\.audio/);
  assert.match(workspace, /runtimeIssues\.interruption/);
  assert.match(workspace, /stateLabelOverride/);
  assert.match(workspace, /audioIssue\s*=\s*runtimeIssues\.engine\s*\?\?\s*runtimeIssues\.audio/);
  assert.match(workspace, /actualOutput=\{audioIssue\s*\?\s*'原声'/);
  assert.match(recoveryAlert, /if \(!issue\) return null/);
  assert.match(recoveryAlert, /role="alert"/);
  assert.match(recoveryAlert, /loading=\{recovering\}/);
  assert.match(recoveryAlert, /onRecover/);
});

test("audio feature shortcuts live below final output and before PortAudio and speech", async () => {
  const [app, output, portAudioPanel] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/OutputColumn.jsx", "utf8"),
    readFile("src/components/PortAudioDevicePanel.jsx", "utf8"),
  ]);
  const outputUsage = app.match(/<OutputColumn[\s\S]*?\/>/)?.[0] ?? "";
  const finalWindowIndex = output.indexOf("最终效果窗口");
  const audioFeaturesIndex = output.indexOf("声音功能");
  const portAudioIndex = output.indexOf("PortAudio 设备");
  const speechIndex = output.indexOf("话术功能");

  assert.ok(finalWindowIndex >= 0 && finalWindowIndex < audioFeaturesIndex, "audio feature card must follow final output");
  assert.ok(audioFeaturesIndex < portAudioIndex, "audio feature card must precede PortAudio");
  assert.ok(portAudioIndex >= 0, "OutputColumn is missing the PortAudio device card");
  assert.ok(speechIndex > portAudioIndex, "PortAudio device card must precede speech features");
  for (const label of ["实际出口", "Host API", "输出设备", "内存缓冲", "处理状态", "应用设置"]) {
    assert.match(portAudioPanel, new RegExp(label), `PortAudio device card missing: ${label}`);
  }
  assert.match(outputUsage, /portAudioStatus=/);
  assert.match(outputUsage, /onPortAudioChange=/);
  assert.match(outputUsage, /onPortAudioApply=/);
  assert.match(outputUsage, /onPortAudioTestTone=/);
  assert.match(portAudioPanel, /onClick=\{onTestTone\}>播放测试音/);
  assert.match(portAudioPanel, /Select/);
  assert.match(portAudioPanel, /InputNumber/);
  assert.match(portAudioPanel, /onChange/);
  assert.match(portAudioPanel, /onApply/);
  assert.match(portAudioPanel, /128[\s\S]*2048/);
  assert.match(portAudioPanel, /status\.dirty/);
  assert.match(app, /next\.hostApi !== next\.appliedHostApi/);
  assert.match(app, /next\.outputDeviceId !== next\.appliedOutputDeviceId/);
  assert.match(app, /next\.memoryBufferKib !== next\.appliedMemoryBufferKib/);
  assert.match(portAudioPanel, /onTestTone/);
});

test("playback pool operations reject partial edits and keep stable order", async () => {
  const {
    appendPlaybackPool,
    movePlaybackPoolItem,
    preparePoolFiles,
    removePlaybackPoolItem,
    replacePlaybackPoolItem,
  } = await import("../src/data/playback-pool-operations.js");
  const files = [
    { name: "A.mp4", size: 1024, lastModified: 1 },
    { name: "B.mov", size: 2048, lastModified: 2 },
  ];
  const prepared = preparePoolFiles(files);
  assert.equal(prepared.ok, true);
  assert.deepEqual(prepared.items.map(({ name }) => name), ["A.mp4", "B.mov"]);

  const duplicate = appendPlaybackPool(prepared.items, [prepared.items[0]]);
  assert.equal(duplicate.ok, false);
  assert.deepEqual(prepared.items.map(({ name }) => name), ["A.mp4", "B.mov"]);

  const moved = movePlaybackPoolItem(prepared.items, prepared.items[1].id, 0);
  assert.deepEqual(moved.pool.map(({ name }) => name), ["B.mov", "A.mp4"]);

  const replacement = preparePoolFiles([{ name: "C.mkv", size: 4096, lastModified: 3 }]).items[0];
  const replaced = replacePlaybackPoolItem(moved.pool, prepared.items[0].id, replacement);
  assert.deepEqual(replaced.pool.map(({ name }) => name), ["B.mov", "C.mkv"]);

  const removed = removePlaybackPoolItem(replaced.pool, prepared.items[1].id);
  assert.deepEqual(removed.pool.map(({ name }) => name), ["C.mkv"]);
  assert.equal(preparePoolFiles([{ name: "bad.txt", size: 10 }]).ok, false);
  assert.equal(preparePoolFiles([{ name: "empty.mp4", size: 0 }]).ok, false);
});

function rustStructFields(source, structName) {
  const marker = `pub struct ${structName} {`;
  const start = source.indexOf(marker);
  assert.notEqual(start, -1, `missing Rust parameter struct: ${structName}`);
  const bodyStart = start + marker.length;
  const bodyEnd = source.indexOf("\n}", bodyStart);
  assert.notEqual(bodyEnd, -1, `unterminated Rust parameter struct: ${structName}`);
  return [...source.slice(bodyStart, bodyEnd).matchAll(/^\s*pub\s+([a-z0-9_]+)\s*:/gm)]
    .map((match) => match[1]);
}

test("prototype lists every field from the current formal media parameter model", async () => {
  const [rustModel, prototypeData] = await Promise.all([
    readFile(new URL("../../../desktop/src-tauri/src/media_effect_params.rs", import.meta.url), "utf8"),
    import("../src/data/prototype-data.js"),
  ]);
  const categories = [
    ["普通视频", "VideoEffectParams", "VIDEO_PARAMETERS"],
    ["高级视觉", "AdvancedEffectParams", "ADVANCED_PARAMETERS"],
    ["普通声音", "AudioEffectParams", "AUDIO_PARAMETERS"],
  ];
  const gaps = {};

  for (const [label, structName, exportName] of categories) {
    const expected = rustStructFields(rustModel, structName);
    const actual = prototypeData[exportName].map((parameter) => parameter.key);
    const formalFields = actual.map((field) => field.startsWith("band_weights.") ? "band_weights" : field);
    const actualSet = new Set(formalFields);
    const expectedSet = new Set(expected);
    const missing = expected.filter((field) => !actualSet.has(field));
    const unexpected = formalFields.filter((field) => !expectedSet.has(field));
    const duplicates = actual.filter((field, index) => actual.indexOf(field) !== index);

    if (missing.length || unexpected.length || duplicates.length) {
      gaps[label] = { missing, unexpected, duplicates };
    }
  }

  assert.deepEqual(gaps, {}, "prototype parameter groups must match the formal Rust structs field-for-field");
});

test("fixed visual band weights render as twelve independent parameters", async () => {
  const [prototypeData, workspace, css] = await Promise.all([
    import("../src/data/prototype-data.js"),
    readFile("src/components/ParameterWorkspace.jsx", "utf8"),
    readFile("src/styles.css", "utf8"),
  ]);
  const expectedFrequencies = [65, 92, 131, 188, 267, 381, 544, 777, 1110, 1585, 2263, 20000];
  const bandParameters = prototypeData.ADVANCED_PARAMETERS.filter(({ key }) => key.startsWith("band_weights."));

  assert.equal(bandParameters.length, 12);
  assert.deepEqual(bandParameters.map(({ key }) => Number(key.split(".")[1])), expectedFrequencies);
  assert.equal(new Set(bandParameters.map(({ key }) => key)).size, expectedFrequencies.length);
  for (const [index, parameter] of bandParameters.entries()) {
    const expectedFrequency = expectedFrequencies[index];
    const expectedWeight = prototypeData.VISUAL_BAND_WEIGHTS[index][1];
    assert.equal(parameter.key, `band_weights.${expectedFrequency}`);
    assert.equal(parameter.sourceKey, "band_weights");
    assert.equal(parameter.fieldPath, `advanced.band_weights.${expectedFrequency}`);
    assert.equal(parameter.label, `${expectedFrequency} Hz 视觉频段权重`);
    assert.equal(parameter.value, `${expectedWeight.toFixed(2)} 倍`);
    assert.ok(Number.isFinite(expectedWeight) && expectedWeight >= 0.5 && expectedWeight <= 1.5);
    assert.equal(parameter.type, undefined);
    assert.equal(parameter.bands, undefined);
  }
  assert.equal(prototypeData.ADVANCED_PARAMETERS.some(({ key }) => key === "band_weights"), false);
  assert.doesNotMatch(workspace, /isFrequencyCard|parameter-row--frequency|parameter-card__frequency-grid/);
  assert.doesNotMatch(css, /parameter-row--frequency|parameter-card__frequency-grid|parameter-card__frequency-item/);
});

function cssRule(css, selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = css.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  assert.ok(match, `missing CSS rule: ${selector}`);
  return match[1];
}

test("ordinary parameter boxes stay compact while labels and values remain on one line", async () => {
  const css = await readFile("src/styles.css", "utf8");
  const row = cssRule(css, ".parameter-row");
  const grid = cssRule(css, ".parameter-section__grid");
  const label = cssRule(css, ".parameter-card__label");
  const value = cssRule(css, ".parameter-card__value");
  const height = Number(row.match(/height:\s*([\d.]+)px/)?.[1]);
  const verticalPadding = Number(row.match(/padding:\s*([\d.]+)px/)?.[1]);
  const labelFontSize = Number(label.match(/font-size:\s*([\d.]+)px/)?.[1]);
  const valueFontSize = Number(value.match(/font-size:\s*([\d.]+)px/)?.[1]);

  assert.ok(Number.isFinite(height) && height < 34, `parameter row height must be < 34px, received ${height}px`);
  assert.ok(Number.isFinite(verticalPadding) && verticalPadding <= 8, `parameter row vertical padding must be <= 8px, received ${verticalPadding}px`);
  const minimumWidth = Number(grid.match(/min\(([\d.]+)px/)?.[1]);
  assert.ok(Number.isFinite(minimumWidth) && minimumWidth <= 132, `parameter grid minimum width must be <= 132px, received ${minimumWidth}px`);
  assert.ok(Number.isFinite(valueFontSize) && valueFontSize >= 11, `parameter value font size must be >= 11px, received ${valueFontSize}px`);
  assert.ok(Number.isFinite(labelFontSize) && valueFontSize > labelFontSize, `parameter value font size must exceed label font size, received ${valueFontSize}px vs ${labelFontSize}px`);
  assert.match(label, /white-space:\s*nowrap/);
  assert.match(value, /white-space:\s*nowrap/);
});

test("only the screenshot cycle and audio status blocks use the tighter density", async () => {
  const [workspace, audioPanel, css] = await Promise.all([
    readFile("src/components/ParameterWorkspace.jsx", "utf8"),
    readFile("src/components/AudioProcessingPanel.jsx", "utf8"),
    readFile("src/styles.css", "utf8"),
  ]);
  const cycleCard = cssRule(css, ".cycle-status-card");
  const audioStatus = cssRule(css, ".audio-processing-panel");
  const waveform = cssRule(css, ".diagnostic-waveform");

  assert.match(cycleCard, /padding:\s*6px 8px/);
  assert.match(cycleCard, /margin-bottom:\s*6px/);
  assert.match(audioStatus, /padding:\s*6px 8px/);
  assert.match(audioStatus, /margin-bottom:\s*6px/);
  assert.match(css, /\.audio-processing-panel \.output-facts-list\s*\{[^}]*display:\s*grid[^}]*grid-template-columns:\s*repeat\(3,\s*minmax\(0,\s*1fr\)\)/s);
  assert.match(css, /\.audio-processing-panel \.output-facts-list > div\s*\{[^}]*display:\s*flex[^}]*align-items:\s*center[^}]*min-width:\s*0/s);
  assert.match(css, /\.audio-processing-panel \.output-facts-list dt\s*\{[^}]*white-space:\s*nowrap/s);
  assert.match(css, /\.audio-processing-panel \.output-facts-list > div \+ div\s*\{[^}]*margin-top:\s*0/);
  assert.match(css, /\.cycle-status-card__progress\s*\{[^}]*line-height:\s*1/);
  assert.match(waveform, /height:\s*48px/);
  assert.match(workspace, /size=\{\["100%", 4\]\}/);
  assert.match(audioPanel, /audio-processing-panel/);
  assert.doesNotMatch(css, /\.panel-section\s*\{[^}]*padding:\s*(?:[0-9]|10)px/);
});

test("all feature drawers share the interruption drawer structure", async () => {
  const [shell, advancedDrawer, interruptionDrawer, fixedSpeechDrawer, css] = await Promise.all([
    readFile("src/components/FeatureDrawerShell.jsx", "utf8"),
    readFile("src/components/AdvancedAudioDrawer.jsx", "utf8"),
    readFile("src/components/InterruptionDrawer.jsx", "utf8"),
    readFile("src/components/FixedSpeechDrawer.jsx", "utf8"),
    readFile("src/styles.css", "utf8"),
  ]);
  const contracts = [
    ["advanced audio", advancedDrawer, "advanced-audio-drawer"],
    ["interruption", interruptionDrawer, "interruption-drawer"],
    ["fixed speech", fixedSpeechDrawer, "fixed-speech-drawer"],
  ];

  for (const [label, source, modifierClass] of contracts) {
    assert.match(source, /<FeatureDrawerShell/, `${label} drawer must use the shared drawer shell`);
    assert.match(source, new RegExp(`modifierClass=["']${modifierClass}["']`), `${label} drawer must keep a dedicated modifier class`);
    assert.match(source, /title=["'][^"']+["'][\s\S]*description=["'][^"']+["']/, `${label} drawer needs a two-line title`);
    assert.match(source, /summaryLabel=["'][^"']+["'][\s\S]*summary=\{/, `${label} drawer needs a live status summary`);
    assert.match(source, /<FeatureDrawerSection/, `${label} drawer content must be grouped with Card sections`);
    assert.match(source, /footer=\{/);
  }

  assert.match(shell, /rootClassName=\{`feature-drawer \$\{modifierClass\}`\}/);
  assert.match(shell, /feature-drawer__title[\s\S]*<strong>[\s\S]*<Typography\.Text/);
  assert.match(shell, /feature-drawer__summary[\s\S]*aria-live="polite"/);
  assert.match(shell, /<Card[\s\S]*feature-drawer__section/);
  assert.match(shell, /body:\s*\{[^}]*overflowY:\s*"auto"/);
  assert.match(shell, /feature-drawer__footer/);

  for (const sharedClass of ["title", "summary", "section", "footer"]) {
    assert.match(css, new RegExp(`\\.feature-drawer__${sharedClass}\\b`), `missing shared feature drawer style: ${sharedClass}`);
  }
});

test("advanced audio and interruption drawers mirror the current desktop controls", async () => {
  const [app, featureDrawers, advancedDrawer, interruptionDrawer, css] = await Promise.all([
    readFile("src/App.jsx", "utf8"),
    readFile("src/components/FeatureDrawers.jsx", "utf8"),
    readFile("src/components/AdvancedAudioDrawer.jsx", "utf8"),
    readFile("src/components/InterruptionDrawer.jsx", "utf8"),
    readFile("src/styles.css", "utf8"),
  ]);

  for (const label of [
    "处理与输出",
    "多轨与预设", "多轨合并", "最少随机轨数", "最多随机轨数", "声音参数值预设",
    "音频参数状态", "缓存管理", "删除已生成缓存", "重新生成本周期参数", "应用声音参数",
  ]) assert.match(advancedDrawer, new RegExp(label), `advanced audio drawer missing: ${label}`);
  for (const label of ["p21", "p22"]) {
    assert.match(advancedDrawer, new RegExp(label), `advanced audio option missing: ${label}`);
  }
  assert.doesNotMatch(advancedDrawer, /PortAudio 设备|Host API|输出设备|内存缓冲|播放测试音/);
  assert.doesNotMatch(advancedDrawer, /onPreview|previewTestTone|frequencyHz:\s*440|HOST_API_OPTIONS|DEFAULT_DEVICE_OPTIONS/);
  const advancedDrawerUsage = featureDrawers.match(/<AdvancedAudioDrawer[\s\S]*?\/>/)?.[0] ?? "";
  assert.doesNotMatch(featureDrawers, /onAdvancedAudioPreview/);
  assert.doesNotMatch(advancedDrawerUsage, /onPreview=/);
  assert.match(advancedDrawer, /应用声音参数/);
  assert.doesNotMatch(advancedDrawer, /音视频同时应用|videoProcessingEnabled|测试音频率|测试音时长|测试音振幅|恢复默认|>取消</);

  for (const label of [
    "启用状态", "音频来源", "插话音频目录", "独立声音轨", "音轨选择方式", "固定选择",
    "从所选音轨随机", "随机多轨合一", "插话声音预设", "每次插话重新随机", "按周期更新",
    "变化周期最小值", "变化周期最大值", "触发与混音", "最小间隔", "最大间隔",
    "插话音量", "原声压低", "原声压低过渡", "原声恢复过渡", "保存插话配置",
  ]) assert.match(interruptionDrawer, new RegExp(label), `interruption drawer missing: ${label}`);
  assert.match(interruptionDrawer, /p21[\s\S]*p22/);
  assert.doesNotMatch(interruptionDrawer, /试听一次插话|>取消</);
  assert.match(interruptionDrawer, /modifierClass="interruption-drawer"/);
  assert.match(interruptionDrawer, /title="随机插话"[\s\S]*递归扫描本地音频目录/);
  assert.match(interruptionDrawer, /summaryLabel="随机插话状态概览"[\s\S]*summary=\{/);
  assert.match(interruptionDrawer, /interruption-drawer__parameter-groups[\s\S]*触发间隔[\s\S]*音量与原声压低[\s\S]*过渡时间/);
  assert.match(css, /\.interruption-drawer__checkbox-grid\s*\{[^}]*repeat\(4,/);
  assert.match(css, /\.feature-drawer__footer\s*\{[^}]*justify-content:\s*flex-end/);

  assert.match(featureDrawers, /AdvancedAudioDrawer/);
  assert.match(featureDrawers, /InterruptionDrawer/);
  assert.doesNotMatch(featureDrawers, /function AdvancedAudioContent|function InterruptionContent/);
  for (const field of ["audioMixEnabled", "audioMixPickMin", "audioMixPickMax", "audioVariationMode", "audioVariationPeriodMinMs", "audioVariationPeriodMaxMs"]) {
    assert.match(app, new RegExp(field), `prototype state missing: ${field}`);
  }
});

test("interruption variation period is edited in seconds while state remains milliseconds", async () => {
  const drawer = await readFile("src/components/InterruptionDrawer.jsx", "utf8");

  assert.match(drawer, /变化周期最小值[\s\S]*?unit="秒"[\s\S]*?min=\{1\}[\s\S]*?max=\{60\}[\s\S]*?value=\{seconds\(value\.audioVariationPeriodMinMs\)\}/);
  assert.match(drawer, /audioVariationPeriodMinMs[\s\S]*?Math\.round\(nextValue \* 1_000\)/);
  assert.match(drawer, /变化周期最大值[\s\S]*?unit="秒"[\s\S]*?value=\{seconds\(value\.audioVariationPeriodMaxMs\)\}/);
  assert.doesNotMatch(drawer, /变化周期最小值[\s\S]{0,500}?unit="ms"/);
  assert.match(drawer, /变化周期必须为 1–60 秒/);
});
