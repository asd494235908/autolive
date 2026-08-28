import { useEffect, useMemo, useState } from "react";
import { App as AntApp, ConfigProvider, theme } from "antd";
import { AppShell } from "./components/AppShell.jsx";
import { FeatureDrawers } from "./components/FeatureDrawers.jsx";
import { OutputColumn } from "./components/OutputColumn.jsx";
import { ParameterWorkspace } from "./components/ParameterWorkspace.jsx";
import { PlaybackColumn } from "./components/PlaybackColumn.jsx";
import { DEFAULT_AUDIO_PRESET_IDS } from "./data/audio-preset-options.js";
import {
  appendPlaybackPool,
  movePlaybackPoolItem,
  preparePoolFiles,
  removePlaybackPoolItem,
  replacePlaybackPool,
  replacePlaybackPoolItem,
} from "./data/playback-pool-operations.js";

const INITIAL_POOL = [
  { id: "source-1", name: "产品演示_竖屏01.mp4", meta: "09:59 · 720×1280 · 29fps · 110.6 MB", status: "ready" },
  { id: "source-2", name: "门店讲解_横屏02.mov", meta: "06:42 · 1920×1080 · 30fps · 184.2 MB", status: "ready" },
  { id: "source-3", name: "功能介绍_竖屏03.mkv", meta: "04:18 · 1080×1920 · 30fps · 96.8 MB", status: "ready" },
];

const DEFAULT_ADVANCED_AUDIO = {
  mixEnabled: false,
  mixPickMin: 1,
  mixPickMax: 2,
  selectedPresetIds: DEFAULT_AUDIO_PRESET_IDS,
};

const DEFAULT_PORTAUDIO_SETTINGS = {
  hostApi: "wasapi",
  outputDeviceId: "wasapi-speakers",
  memoryBufferKib: 1024,
  appliedHostApi: "wasapi",
  appliedOutputDeviceId: "wasapi-speakers",
  appliedMemoryBufferKib: 1024,
  dirty: false,
};

const DEFAULT_INTERRUPTION = {
  enabled: true,
  directory: "D:/Audio/Interludes",
  audioSelectionMode: "random",
  audioFixedPresetId: "p01",
  audioPresetIds: DEFAULT_AUDIO_PRESET_IDS,
  audioMixEnabled: false,
  audioMixPickMin: 1,
  audioMixPickMax: 2,
  audioVariationMode: "periodic",
  audioVariationPeriodMinMs: 8_000,
  audioVariationPeriodMaxMs: 15_000,
  intervalMinMs: 8_000,
  intervalMaxMs: 13_000,
  volumeDb: 0,
  duckingDepthDb: -12,
  duckingAttackMs: 50,
  duckingReleaseMs: 250,
  dirty: false,
};

const DEFAULT_INTERRUPTION_RUNTIME = {
  currentAudioPresetId: "p13",
};

const DEMO_RUNTIME_ISSUES = {
  engine: {
    type: "warning",
    title: "媒体引擎不可用",
    description: "未检测到本地媒体处理组件，视频与普通声音保持原始输出。",
    recoveringDescription: "正在重新检测媒体引擎与本地依赖。",
    recoverLabel: "重新检测",
    secondaryLabel: "修复说明",
  },
  video: {
    type: "error",
    title: "视频处理失败",
    description: "本周期视频参数处理失败，当前已回退原画，播放不会中断。",
    recoveringDescription: "正在重新提交本周期视频处理。",
    recoverLabel: "重试处理",
    secondaryLabel: "恢复默认后重试",
  },
  audio: {
    type: "error",
    title: "声音处理失败",
    description: "普通声音处理失败，当前已保留原声并继续播放。",
    recoveringDescription: "正在重新初始化声音处理链。",
    recoverLabel: "重试处理",
    secondaryLabel: "保留原声",
  },
  interruption: {
    type: "error",
    title: "随机插话失败",
    description: "插话音频目录不可访问，本次插话已跳过，主视频继续播放。",
    recoveringDescription: "正在重新扫描插话目录并检查可读音频。",
    recoverLabel: "重新扫描目录",
    secondaryLabel: "打开插话设置",
  },
};

function getInitialRuntimeIssues() {
  const issueKey = new URLSearchParams(window.location.search).get("issue");
  if (issueKey === "all") return { ...DEMO_RUNTIME_ISSUES };
  return issueKey && DEMO_RUNTIME_ISSUES[issueKey]
    ? { [issueKey]: DEMO_RUNTIME_ISSUES[issueKey] }
    : {};
}

function formatTime(percent, totalSeconds = 599) {
  const value = Math.round((Math.max(0, Math.min(100, percent)) / 100) * totalSeconds);
  return `${String(Math.floor(value / 60)).padStart(2, "0")}:${String(value % 60).padStart(2, "0")}`;
}

function PrototypeApp() {
  const { message } = AntApp.useApp();
  const [pool, setPool] = useState(INITIAL_POOL);
  const [currentIndex, setCurrentIndex] = useState(0);
  const [playbackStatus, setPlaybackStatus] = useState("playing");
  const [playbackActionBusy, setPlaybackActionBusy] = useState(null);
  const [progress, setProgress] = useState(63);
  const [volume, setVolume] = useState(72);
  const [muted, setMuted] = useState(false);
  const [poolCycle, setPoolCycle] = useState(1);
  const [cycleRange, setCycleRange] = useState({ videoMin: 8, videoMax: 15, audioMin: 3, audioMax: 5 });
  const [videoEnabled, setVideoEnabled] = useState(true);
  const [audioEnabled, setAudioEnabled] = useState(true);
  const [windowOpen, setWindowOpen] = useState(true);
  const [outputMode, setOutputMode] = useState("独立窗口");
  const [activeDrawer, setActiveDrawer] = useState(null);
  const [advancedAudio, setAdvancedAudio] = useState(DEFAULT_ADVANCED_AUDIO);
  const [portAudioSettings, setPortAudioSettings] = useState(DEFAULT_PORTAUDIO_SETTINGS);
  const [interruption, setInterruption] = useState(DEFAULT_INTERRUPTION);
  const [interruptionRuntime, setInterruptionRuntime] = useState(DEFAULT_INTERRUPTION_RUNTIME);
  const [runtimeIssues, setRuntimeIssues] = useState(getInitialRuntimeIssues);
  const [recoveryBusy, setRecoveryBusy] = useState(null);
  const [fixedSpeech, setFixedSpeech] = useState({ title: "开场欢迎", text: "欢迎进入直播间，感谢大家的支持。" });

  useEffect(() => {
    if (playbackStatus !== "playing" || pool.length === 0) return undefined;
    const timer = window.setInterval(() => {
      setProgress((current) => {
        if (current < 99.5) return current + 0.5;
        setCurrentIndex((index) => {
          const nextIndex = (index + 1) % pool.length;
          if (nextIndex === 0) setPoolCycle((cycle) => cycle + 1);
          return nextIndex;
        });
        return 0;
      });
    }, 500);
    return () => window.clearInterval(timer);
  }, [playbackStatus, pool.length]);

  const currentSource = pool[currentIndex] ?? null;
  const processingChain = useMemo(() => {
    const videoIssue = runtimeIssues.engine ?? runtimeIssues.video;
    const audioIssue = runtimeIssues.engine ?? runtimeIssues.audio;
    const videoRecovering = recoveryBusy === "engine" || recoveryBusy === "video";
    const audioRecovering = recoveryBusy === "engine" || recoveryBusy === "audio";
    return [
      { label: "素材", detail: currentSource ? "已就绪" : "等待导入", state: currentSource ? "done" : "idle" },
      {
        label: "视频",
        detail: videoRecovering ? "恢复中" : videoIssue ? "失败回退" : videoEnabled ? "处理中" : "已旁路",
        state: videoRecovering ? "active" : videoIssue ? "error" : videoEnabled ? "active" : "idle",
      },
      {
        label: "声音",
        detail: audioRecovering ? "恢复中" : audioIssue ? "保留原声" : audioEnabled ? "PortAudio" : "保留原声",
        state: audioRecovering ? "active" : audioIssue ? "error" : audioEnabled ? "active" : "idle",
      },
      { label: "输出", detail: windowOpen ? "已连接" : "等待窗口", state: windowOpen ? "done" : "idle" },
    ];
  }, [audioEnabled, currentSource, recoveryBusy, runtimeIssues, videoEnabled, windowOpen]);

  function changeSetting(setter, field, value) {
    setter((current) => ({ ...current, [field]: value }));
  }

  async function handlePlaybackAction(action) {
    if (playbackActionBusy) return;
    setPlaybackActionBusy(action);
    try {
      await new Promise((resolve) => window.setTimeout(resolve, 240));
      if (action === "play" || action === "resume") {
        setPlaybackStatus("playing");
        setWindowOpen(true);
        message.success(action === "play" ? "已开始顺序播放" : "已继续播放");
        return;
      }
      if (action === "pause") setPlaybackStatus("paused");
      if (action === "stop") {
        setPlaybackStatus("stopped");
        setProgress(0);
      }
    } finally {
      setPlaybackActionBusy(null);
    }
  }

  function clearRuntimeIssue(issueKey) {
    setRuntimeIssues((current) => {
      const next = { ...current };
      delete next[issueKey];
      return next;
    });
  }

  async function handleRuntimeRecovery(issueKey) {
    if (recoveryBusy || !runtimeIssues[issueKey]) return;
    setRecoveryBusy(issueKey);
    try {
      await new Promise((resolve) => window.setTimeout(resolve, 650));
      clearRuntimeIssue(issueKey);
      message.success(`${DEMO_RUNTIME_ISSUES[issueKey].title}已恢复`);
    } finally {
      setRecoveryBusy(null);
    }
  }

  function handleRecoverySecondary(issueKey) {
    if (issueKey === "engine") {
      message.info("确认本地媒体组件可执行后，点击“重新检测”完成恢复");
      return;
    }
    if (issueKey === "video") {
      clearRuntimeIssue(issueKey);
      message.success("已恢复视频默认值并重新启用处理");
      return;
    }
    if (issueKey === "audio") {
      setAudioEnabled(false);
      clearRuntimeIssue(issueKey);
      message.info("已关闭普通声音处理并保留原声");
      return;
    }
    if (issueKey === "interruption") setActiveDrawer("interruption");
  }

  function commitPoolResult(result, successMessage) {
    if (!result.ok) {
      message.error(result.error);
      return;
    }
    if (!result.changed) {
      message.info("播放池没有变化");
      return;
    }
    setPool(result.pool);
    setCurrentIndex(0);
    setProgress(0);
    setPoolCycle(1);
    setPlaybackStatus(result.pool.length ? "ready" : "stopped");
    message.success(successMessage);
  }

  function prepareFiles(files) {
    const prepared = preparePoolFiles(files);
    if (!prepared.ok) message.error(prepared.error);
    return prepared;
  }

  function handlePoolReplaceAll(files) {
    const prepared = prepareFiles(files);
    if (prepared.ok) commitPoolResult(replacePlaybackPool(prepared.items), `已用 ${prepared.items.length} 个文件原子替换播放池`);
  }

  function handlePoolAppend(files) {
    const prepared = prepareFiles(files);
    if (prepared.ok) commitPoolResult(appendPlaybackPool(pool, prepared.items), `已追加 ${prepared.items.length} 个视频`);
  }

  function handlePoolReplaceItem(itemId, file) {
    const prepared = prepareFiles([file]);
    if (prepared.ok) commitPoolResult(replacePlaybackPoolItem(pool, itemId, prepared.items[0]), "已替换播放池条目");
  }

  function handleSave(drawer) {
    if (drawer === "interruption") {
      setInterruption((current) => ({ ...current, dirty: false }));
      setInterruptionRuntime((current) => {
        const selectedIds = interruption.audioSelectionMode === "fixed"
          ? [interruption.audioFixedPresetId]
          : interruption.audioPresetIds;
        return {
          currentAudioPresetId: selectedIds.includes(current.currentAudioPresetId)
            ? current.currentAudioPresetId
            : selectedIds[0] ?? null,
        };
      });
    }
    setActiveDrawer(null);
    if (drawer === "fixedSpeech") message.success("固定话术已保存到本地原型");
    else if (drawer === "interruption") message.success("插话配置保存成功");
    else message.success("声音参数保存成功");
  }

  return (
    <AppShell onWindowAction={(action) => message.info(`${action}仅用于桌面原型演示`)}>
      <div className="prototype-ribbon" role="note">交互原型 · 运行数据为本地模拟</div>
      <div className="prototype-workspace">
        <PlaybackColumn
          pool={pool}
          currentIndex={currentIndex}
          playbackStatus={playbackStatus}
          playbackActionBusy={playbackActionBusy}
          progress={progress}
          cycleCount={poolCycle}
          currentTime={formatTime(progress)}
          duration="09:59"
          volume={volume}
          muted={muted}
          onPoolReplaceAll={handlePoolReplaceAll}
          onPoolAppend={handlePoolAppend}
          onPoolReplaceItem={handlePoolReplaceItem}
          onPoolMove={(itemId, targetIndex) => commitPoolResult(movePlaybackPoolItem(pool, itemId, targetIndex), "播放顺序已更新")}
          onPoolRemove={(itemId) => commitPoolResult(removePlaybackPoolItem(pool, itemId), "播放池条目已删除")}
          onPoolClear={() => commitPoolResult({ ok: true, changed: pool.length > 0, pool: [] }, "播放池已清空")}
          onPlaybackAction={handlePlaybackAction}
          onSeek={setProgress}
          onVolumeChange={setVolume}
          onMuteToggle={() => setMuted((value) => !value)}
          onPictureInPicture={() => {
            setOutputMode("画中画");
            setWindowOpen(true);
            message.success("已切换到画中画演示模式");
          }}
        />

        <ParameterWorkspace
          videoEnabled={videoEnabled}
          audioEnabled={audioEnabled}
          isPlaying={playbackStatus === "playing"}
          onToggleVideo={setVideoEnabled}
          onToggleAudio={setAudioEnabled}
          onResetVideo={() => message.success("视频参数已恢复为正式默认值")}
          cycleRange={cycleRange}
          onCycleRangeChange={(field, value) => changeSetting(setCycleRange, field, value)}
          interruption={interruption}
          interruptionRuntime={interruptionRuntime}
          runtimeIssues={runtimeIssues}
          recoveryBusy={recoveryBusy}
          onRecoverIssue={handleRuntimeRecovery}
          onRecoverySecondary={handleRecoverySecondary}
        />

        <OutputColumn
          windowOpen={windowOpen}
          outputMode={outputMode}
          processingChain={processingChain}
          portAudioStatus={{
            ...portAudioSettings,
            actualOutput: audioEnabled ? "PortAudio" : "WebView",
            processingStatus: audioEnabled ? "运行中" : "已旁路",
          }}
          onPortAudioChange={(field, value) => setPortAudioSettings((current) => {
            const next = {
              ...current,
              [field]: value,
              ...(field === "hostApi" ? { outputDeviceId: value === "all" ? "default" : null } : {}),
            };
            return {
              ...next,
              dirty: next.hostApi !== next.appliedHostApi
                || next.outputDeviceId !== next.appliedOutputDeviceId
                || next.memoryBufferKib !== next.appliedMemoryBufferKib,
            };
          })}
          onPortAudioApply={() => {
            setPortAudioSettings((current) => ({
              ...current,
              appliedHostApi: current.hostApi,
              appliedOutputDeviceId: current.outputDeviceId,
              appliedMemoryBufferKib: current.memoryBufferKib,
              dirty: false,
            }));
            message.success("PortAudio 设备设置已应用");
          }}
          onPortAudioTestTone={() => message.info("正在播放 440 Hz / 400 ms 本地测试音")}
          localStatus={[
            {
              label: "媒体引擎",
              value: recoveryBusy === "engine" ? "恢复中" : runtimeIssues.engine ? "不可用" : "就绪",
              tone: recoveryBusy === "engine" ? "warning" : runtimeIssues.engine ? "error" : "success",
            },
            {
              label: "视频处理",
              value: recoveryBusy === "video" ? "恢复中" : runtimeIssues.video ? "失败回退" : videoEnabled ? "运行中" : "已旁路",
              tone: recoveryBusy === "video" ? "warning" : runtimeIssues.video ? "error" : videoEnabled ? "success" : "muted",
            },
            {
              label: "声音处理",
              value: recoveryBusy === "audio" ? "恢复中" : runtimeIssues.audio ? "失败回退" : audioEnabled ? "运行中" : "已旁路",
              tone: recoveryBusy === "audio" ? "warning" : runtimeIssues.audio ? "error" : audioEnabled ? "success" : "muted",
            },
            {
              label: "随机插话",
              value: recoveryBusy === "interruption" ? "恢复中" : runtimeIssues.interruption ? "失败" : interruption.enabled ? "就绪" : "未启用",
              tone: recoveryBusy === "interruption" ? "warning" : runtimeIssues.interruption ? "error" : interruption.enabled ? "success" : "muted",
            },
            { label: "最终窗口", value: windowOpen ? "已连接" : "未连接", tone: windowOpen ? "success" : "muted" },
            { label: "本地源", value: currentSource ? "可用" : "未导入", tone: currentSource ? "success" : "muted" },
          ]}
          onOpenWindow={() => {
            setWindowOpen(true);
            message.success("已打开并聚焦单实例最终效果窗口");
          }}
          onOutputModeChange={setOutputMode}
          onOpenDrawer={setActiveDrawer}
        />
      </div>

      <FeatureDrawers
        activeDrawer={activeDrawer}
        advancedAudio={advancedAudio}
        interruption={interruption}
        fixedSpeech={fixedSpeech}
        audioEnabled={audioEnabled}
        onClose={() => setActiveDrawer(null)}
        onAudioEnabledChange={setAudioEnabled}
        onAdvancedAudioChange={(field, value) => changeSetting(setAdvancedAudio, field, value)}
        onAdvancedAudioRegenerate={() => {
          setAdvancedAudio(DEFAULT_ADVANCED_AUDIO);
          message.success("已重新生成本周期声音参数");
        }}
        onAdvancedAudioCleanup={() => {
          setAdvancedAudio((current) => ({ ...current, cleanupSummary: "已删除 3 个文件" }));
          message.success("缓存清理完成");
        }}
        onInterruptionChange={(field, value) => setInterruption((current) => ({ ...current, [field]: value, dirty: true }))}
        onFixedSpeechChange={(field, value) => changeSetting(setFixedSpeech, field, value)}
        onFixedSpeechPreview={() => message.info("正在使用本机系统语音试听")}
        onSave={handleSave}
      />
    </AppShell>
  );
}

export function App() {
  return (
    <ConfigProvider
      theme={{
        algorithm: theme.darkAlgorithm,
        token: {
          colorPrimary: "#31d7aa",
          colorInfo: "#5ea2ff",
          colorBgBase: "#0b0b0f",
          colorBgContainer: "#17171c",
          colorBorder: "#303139",
          borderRadius: 8,
          controlHeight: 30,
          fontFamily: 'Inter, "Segoe UI", "Microsoft YaHei UI", sans-serif',
          fontSize: 12,
        },
      }}
    >
      <AntApp><PrototypeApp /></AntApp>
    </ConfigProvider>
  );
}
