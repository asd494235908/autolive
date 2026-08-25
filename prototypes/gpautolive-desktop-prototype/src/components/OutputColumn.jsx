import { useEffect, useRef } from "react";
import {
  AudioOutlined,
  CheckCircleOutlined,
  ClockCircleOutlined,
  FolderOpenOutlined,
  LoadingOutlined,
  MessageOutlined,
  SettingOutlined,
  SoundOutlined,
} from "@ant-design/icons";
import { Button, Segmented, Switch, Tag } from "antd";

const DEFAULT_WAVEFORM = [
  0.08, 0.22, -0.14, 0.31, -0.38, 0.16, 0.42, -0.24, 0.12, -0.18, 0.28,
  -0.12, 0.47, -0.36, 0.18, -0.29, 0.11, 0.35, -0.21, 0.08, -0.14, 0.24,
  -0.09, 0.52, -0.41, 0.2, -0.25, 0.13, 0.3, -0.18,
];

function DiagnosticWaveform({ values = DEFAULT_WAVEFORM, active = true }) {
  const canvasRef = useRef(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return undefined;

    const draw = () => {
      const width = Math.max(canvas.clientWidth, 280);
      const height = Math.max(canvas.clientHeight, 72);
      const ratio = window.devicePixelRatio || 1;
      canvas.width = width * ratio;
      canvas.height = height * ratio;

      const context = canvas.getContext("2d");
      context.scale(ratio, ratio);
      context.clearRect(0, 0, width, height);
      context.strokeStyle = active ? "#2dd4bf" : "#64748b";
      context.lineWidth = 1.5;
      context.beginPath();
      values.forEach((value, index) => {
        const x = (index / Math.max(values.length - 1, 1)) * width;
        const y = height / 2 - value * height * 0.76;
        if (index === 0) context.moveTo(x, y);
        else context.lineTo(x, y);
      });
      context.stroke();
    };

    draw();
    window.addEventListener("resize", draw);
    return () => window.removeEventListener("resize", draw);
  }, [active, values]);

  return (
    <canvas
      ref={canvasRef}
      className="diagnostic-waveform"
      role="img"
      aria-label={active ? "混音后音频诊断波形" : "当前没有可用的音频诊断波形"}
    />
  );
}

export function OutputColumn({
  windowOpen = false,
  outputMode = "画中画",
  audioProcessingEnabled = true,
  outputDevice = "PortAudio",
  audioStatus = "处理中",
  currentPreset = "13 · 轻弹",
  waveform = DEFAULT_WAVEFORM,
  diagnostics = [],
  processingChain = [],
  onOpenWindow,
  onOutputModeChange,
  onAudioProcessingChange,
  onOpenDrawer,
}) {
  return (
    <aside className="output-column" aria-label="输出与本地处理状态">
      <section className="panel-section output-window-section" aria-labelledby="output-window-title">
        <div className="section-heading">
          <div>
            <h2 id="output-window-title">最终效果窗口</h2>
            <span>{windowOpen ? "窗口已连接" : "窗口尚未打开"}</span>
          </div>
          <Button type="primary" icon={<FolderOpenOutlined />} onClick={onOpenWindow}>
            {windowOpen ? "聚焦" : "打开"}
          </Button>
        </div>
        <Segmented
          block
          aria-label="输出查看模式"
          options={["画中画", "独立窗口"]}
          value={outputMode}
          onChange={onOutputModeChange}
        />
      </section>

      <section className="panel-section audio-output-section" aria-labelledby="audio-output-title">
        <div className="section-heading compact-heading">
          <div>
            <h2 id="audio-output-title">普通声音处理</h2>
            <span>与画面处理独立控制</span>
          </div>
          <Switch
            aria-label="普通声音处理开关"
            checked={audioProcessingEnabled}
            onChange={onAudioProcessingChange}
          />
        </div>
        <dl className="output-facts-list">
          <div><dt>实际出口</dt><dd><Tag color="success">{outputDevice}</Tag></dd></div>
          <div><dt>处理状态</dt><dd><Tag color="processing">{audioStatus}</Tag></dd></div>
          <div><dt>当前预设</dt><dd>{currentPreset}</dd></div>
        </dl>

        <div className="waveform-panel">
          <DiagnosticWaveform values={waveform} active={audioProcessingEnabled} />
          <span>实时 · 混音后 PCM</span>
        </div>

        <dl className="diagnostic-grid" aria-label="音频诊断值">
          {diagnostics.map((item) => (
            <div key={item.label} className="diagnostic-item">
              <dt>{item.label}</dt>
              <dd>{item.value}</dd>
            </div>
          ))}
        </dl>
      </section>

      <section className="panel-section feature-entry-section" aria-labelledby="feature-entry-title">
        <div className="section-heading compact-heading">
          <h2 id="feature-entry-title">声音功能</h2>
        </div>
        <div className="feature-entry-grid">
          <Button icon={<SettingOutlined />} onClick={() => onOpenDrawer?.("advancedAudio")}>
            高级声音
          </Button>
          <Button icon={<AudioOutlined />} onClick={() => onOpenDrawer?.("interruption")}>
            随机插话
          </Button>
          <Button icon={<MessageOutlined />} onClick={() => onOpenDrawer?.("fixedSpeech")}>
            固定话术
          </Button>
        </div>
      </section>

      <section className="panel-section processing-chain-section" aria-labelledby="processing-chain-title">
        <div className="section-heading compact-heading">
          <h2 id="processing-chain-title">实时处理引擎</h2>
          <SoundOutlined aria-hidden="true" />
        </div>
        <ol className="processing-chain-list">
          {processingChain.map((step) => (
            <li key={step.label} className={step.state ? `chain-${step.state}` : undefined}>
              <span className={`chain-state-icon ${step.state ?? "pending"}`} aria-hidden="true">
                {step.state === "done" ? (
                  <CheckCircleOutlined />
                ) : step.state === "active" ? (
                  <LoadingOutlined spin />
                ) : (
                  <ClockCircleOutlined />
                )}
              </span>
              <div><strong>{step.label}</strong><small>{step.detail}</small></div>
            </li>
          ))}
        </ol>
      </section>
    </aside>
  );
}
