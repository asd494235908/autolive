import { useEffect, useMemo, useRef, useState } from "react";
import { App as AntApp, ConfigProvider, theme } from "antd";
import { AppShell } from "./components/AppShell.jsx";
import { FeatureDrawers } from "./components/FeatureDrawers.jsx";
import { OutputColumn } from "./components/OutputColumn.jsx";
import { ParameterWorkspace } from "./components/ParameterWorkspace.jsx";
import { PlaybackColumn } from "./components/PlaybackColumn.jsx";

const INITIAL_POOL = [
  { id: "source-1", name: "产品演示_竖屏01.mp4", meta: "09:59 · 720×1280 · 29fps · 110.6 MB", status: "ready" },
  { id: "source-2", name: "门店讲解_横屏02.mov", meta: "06:42 · 1920×1080 · 30fps · 184.2 MB", status: "ready" },
  { id: "source-3", name: "功能介绍_竖屏03.mkv", meta: "04:18 · 1080×1920 · 30fps · 96.8 MB", status: "ready" },
];

const DIAGNOSTICS = [
  { label: "采样率", value: "44100 Hz" },
  { label: "捕获帧", value: "1536" },
  { label: "RMS", value: "-21.94 dBFS" },
  { label: "峰值", value: "-11.56 dBFS" },
  { label: "噪声底", value: "-36.03 dBFS" },
  { label: "低频截止", value: "180 Hz" },
  { label: "SNR", value: "11.18 dB" },
  { label: "MFCC", value: "12 维" },
];

const WAVEFORM = [
  0.08, 0.2, -0.12, 0.28, -0.34, 0.14, 0.38, -0.2, 0.1, -0.16,
  0.25, -0.1, 0.43, -0.32, 0.16, -0.26, 0.1, 0.31, -0.18, 0.07,
  -0.12, 0.22, -0.08, 0.48, -0.37, 0.18, -0.22, 0.12, 0.28, -0.16,
];

const DEFAULT_ADVANCED_AUDIO = {
  hostApi: "WASAPI",
  device: "default",
  bufferKib: 1024,
  multiTrack: true,
  minTracks: 1,
  maxTracks: 2,
  presetIds: ["p01", "p04", "p07", "p13"],
};

function formatTime(percent, totalSeconds = 599) {
  const value = Math.round((Math.max(0, Math.min(100, percent)) / 100) * totalSeconds);
  return `${String(Math.floor(value / 60)).padStart(2, "0")}:${String(value % 60).padStart(2, "0")}`;
}

function PrototypeApp() {
  const { message } = AntApp.useApp();
  const fileInputRef = useRef(null);
  const [activeView, setActiveView] = useState("home");
  const [pool, setPool] = useState(INITIAL_POOL);
  const [currentIndex, setCurrentIndex] = useState(0);
  const [playbackStatus, setPlaybackStatus] = useState("playing");
  const [progress, setProgress] = useState(63);
  const [volume, setVolume] = useState(72);
  const [muted, setMuted] = useState(false);
  const [poolCycle, setPoolCycle] = useState(1);
  const [cycleRange, setCycleRange] = useState({ videoMin: 8, videoMax: 15, audioMin: 8, audioMax: 15 });
  const [linkedCycles, setLinkedCycles] = useState(false);
  const [videoEnabled, setVideoEnabled] = useState(true);
  const [audioEnabled, setAudioEnabled] = useState(true);
  const [windowOpen, setWindowOpen] = useState(true);
  const [outputMode, setOutputMode] = useState("独立窗口");
  const [activeDrawer, setActiveDrawer] = useState(null);
  const [advancedAudio, setAdvancedAudio] = useState(DEFAULT_ADVANCED_AUDIO);
  const [interruption, setInterruption] = useState({ enabled: false, directory: "", selectionMode: "random", fixedPresetId: "p01", presetIds: ["p01", "p04", "p07", "p13"], minSeconds: 8, maxSeconds: 13, volumeDb: 0, duckingDb: -12, attackMs: 50, releaseMs: 250 });
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
  const processingChain = useMemo(() => [
    { label: "素材", detail: currentSource ? "已就绪" : "等待导入", state: currentSource ? "done" : "idle" },
    { label: "视频", detail: videoEnabled ? "处理中" : "已旁路", state: videoEnabled ? "active" : "idle" },
    { label: "声音", detail: audioEnabled ? "PortAudio" : "保留原声", state: audioEnabled ? "active" : "idle" },
    { label: "输出", detail: windowOpen ? "已连接" : "等待窗口", state: windowOpen ? "done" : "idle" },
  ], [audioEnabled, currentSource, videoEnabled, windowOpen]);

  function changeSetting(setter, field, value) {
    setter((current) => ({ ...current, [field]: value }));
  }

  function handlePlaybackAction(action) {
    if (action === "play" || action === "resume") {
      setPlaybackStatus("playing");
      setWindowOpen(true);
      message.success("最终效果窗口已连接，开始顺序播放");
      return;
    }
    if (action === "pause") setPlaybackStatus("paused");
    if (action === "stop") {
      setPlaybackStatus("stopped");
      setProgress(0);
    }
  }

  function handleFiles(event) {
    const files = [...(event.target.files ?? [])].slice(0, 100);
    event.target.value = "";
    if (!files.length) return;
    setPool(files.map((file, index) => ({
      id: `${file.name}-${file.size}-${index}`,
      name: file.name,
      meta: `待探测 · 本地文件 · ${(file.size / 1024 / 1024).toFixed(1)} MB`,
      status: "ready",
    })));
    setCurrentIndex(0);
    setProgress(0);
    setPlaybackStatus("ready");
    message.success(`已用 ${files.length} 个文件原子替换演示播放池`);
  }

  function handleSave(drawer) {
    setActiveDrawer(null);
    message.success(drawer === "fixedSpeech" ? "固定话术已保存到本地原型" : "保存成功，将从下一周期生效");
  }

  return (
    <AppShell
      activeView={activeView}
      onViewChange={(view) => {
        setActiveView(view);
        if (view !== "home") message.info("当前版本的导航入口共用同一桌面工作台");
      }}
      onWindowAction={(action) => message.info(`${action}仅用于桌面原型演示`)}
    >
      <input
        ref={fileInputRef}
        className="visually-hidden"
        type="file"
        multiple
        accept=".mp4,.mov,.mkv,.avi,.webm,.m4v,.ts,.m2ts,.flv,.wmv,.3gp"
        onChange={handleFiles}
      />
      <div className="prototype-ribbon" role="note">交互原型 · 运行数据为本地模拟</div>
      <div className="prototype-workspace">
        <PlaybackColumn
          pool={pool}
          currentIndex={currentIndex}
          playbackStatus={playbackStatus}
          progress={progress}
          currentTime={formatTime(progress)}
          duration="09:59"
          volume={volume}
          muted={muted}
          cycleRange={cycleRange}
          linkedCycles={linkedCycles}
          localStatus={[
            { label: "媒体引擎", value: "就绪", tone: "success" },
            { label: "音频出口", value: audioEnabled ? "PortAudio" : "WebView", tone: "success" },
            { label: "最终窗口", value: windowOpen ? "已连接" : "未连接", tone: windowOpen ? "success" : "muted" },
            { label: "本地源", value: currentSource ? "可用" : "未导入", tone: currentSource ? "success" : "muted" },
          ]}
          onImport={() => fileInputRef.current?.click()}
          onPlaybackAction={handlePlaybackAction}
          onSeek={setProgress}
          onVolumeChange={setVolume}
          onMuteToggle={() => setMuted((value) => !value)}
          onPictureInPicture={() => {
            setOutputMode("画中画");
            setWindowOpen(true);
            message.success("已切换到画中画演示模式");
          }}
          onCycleRangeChange={(field, value) => changeSetting(setCycleRange, field, value)}
          onLinkedCyclesChange={setLinkedCycles}
        />

        <ParameterWorkspace
          videoEnabled={videoEnabled}
          audioEnabled={audioEnabled}
          isPlaying={playbackStatus === "playing"}
          progress={progress}
          currentSource={currentSource}
          playbackPool={{ current: pool.length ? currentIndex + 1 : 0, total: pool.length, cycle: poolCycle }}
          outputConnected={windowOpen}
          onToggleVideo={setVideoEnabled}
          onResetParameters={() => message.success("参数已恢复为正式默认值")}
          onApply={(scope) => message.success(scope === "both" ? "音视频参数已提交到原型运行快照" : "视频参数已提交到原型运行快照")}
        />

        <OutputColumn
          windowOpen={windowOpen}
          outputMode={outputMode}
          audioProcessingEnabled={audioEnabled}
          outputDevice={audioEnabled ? "PortAudio" : "WebView"}
          audioStatus={audioEnabled ? "处理中" : "已关闭"}
          currentPreset="13 · 轻弹"
          waveform={WAVEFORM}
          diagnostics={DIAGNOSTICS}
          processingChain={processingChain}
          onOpenWindow={() => {
            setWindowOpen(true);
            message.success("已打开并聚焦单实例最终效果窗口");
          }}
          onOutputModeChange={setOutputMode}
          onAudioProcessingChange={setAudioEnabled}
          onOpenDrawer={setActiveDrawer}
        />
      </div>

      <FeatureDrawers
        activeDrawer={activeDrawer}
        advancedAudio={advancedAudio}
        interruption={interruption}
        fixedSpeech={fixedSpeech}
        onClose={() => setActiveDrawer(null)}
        onAdvancedAudioChange={(field, value) => changeSetting(setAdvancedAudio, field, value)}
        onAdvancedAudioReset={() => setAdvancedAudio(DEFAULT_ADVANCED_AUDIO)}
        onInterruptionChange={(field, value) => changeSetting(setInterruption, field, value)}
        onInterruptionPreview={() => message.info("已试听一次插话原声与 duck 包络")}
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
