import { useEffect, useRef } from "react";
import { Tag } from "antd";

const DEFAULT_WAVEFORM = [
  0.08, 0.2, -0.12, 0.28, -0.34, 0.14, 0.38, -0.2, 0.1, -0.16,
  0.25, -0.1, 0.43, -0.32, 0.16, -0.26, 0.1, 0.31, -0.18, 0.07,
  -0.12, 0.22, -0.08, 0.48, -0.37, 0.18, -0.22, 0.12, 0.28, -0.16,
];

function DiagnosticWaveform({
  values = DEFAULT_WAVEFORM,
  active = true,
  ariaLabel,
}) {
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
      aria-label={ariaLabel ?? (active ? "混音后音频诊断波形" : "当前没有可用的音频诊断波形")}
    />
  );
}

export function AudioProcessingPanel({
  ariaLabel = "普通声音状态",
  enabled = true,
  active,
  actualOutput,
  processingStatus,
  processingTone,
  currentPreset = "13 · 轻弹",
  waveformCaption = "实时 · 混音后 PCM",
  waveformAriaLabel,
}) {
  const waveformActive = active ?? enabled;
  const output = actualOutput ?? (enabled ? "PortAudio" : "WebView");
  const status = processingStatus ?? (enabled ? "处理中" : "已关闭");
  const statusTone = processingTone ?? (enabled ? "processing" : "default");

  return (
    <section className="audio-processing-panel" aria-label={ariaLabel}>
      <dl className="output-facts-list">
        <div><dt>实际出口</dt><dd><Tag color="success">{output}</Tag></dd></div>
        <div><dt>处理状态</dt><dd><Tag color={statusTone}>{status}</Tag></dd></div>
        <div><dt>当前预设</dt><dd>{currentPreset}</dd></div>
      </dl>

      <div className="waveform-panel">
        <DiagnosticWaveform active={waveformActive} ariaLabel={waveformAriaLabel} />
        <span>{waveformCaption}</span>
      </div>
    </section>
  );
}
