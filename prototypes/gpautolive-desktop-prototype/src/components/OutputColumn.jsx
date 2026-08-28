import {
  CheckCircleOutlined,
  ClockCircleOutlined,
  CloseCircleOutlined,
  FolderOpenOutlined,
  LoadingOutlined,
  MessageOutlined,
  SettingOutlined,
  SoundOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";
import { Button, Segmented, Tag } from "antd";
import { PortAudioDevicePanel } from "./PortAudioDevicePanel.jsx";

export function OutputColumn({
  windowOpen = false,
  outputMode = "画中画",
  processingChain = [],
  portAudioStatus = {},
  localStatus = [],
  onOpenWindow,
  onOutputModeChange,
  onOpenDrawer,
  onPortAudioChange,
  onPortAudioApply,
  onPortAudioTestTone,
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

      <section className="panel-section feature-entry-section" aria-labelledby="audio-feature-entry-title">
        <div className="section-heading compact-heading">
          <h2 id="audio-feature-entry-title">声音功能</h2>
        </div>
        <div className="feature-entry-grid">
          <Button icon={<SettingOutlined />} onClick={() => onOpenDrawer?.("advancedAudio")}>
            模式修改
          </Button>
          <Button icon={<ThunderboltOutlined />} onClick={() => onOpenDrawer?.("interruption")}>
            随机插话
          </Button>
        </div>
      </section>

      <PortAudioDevicePanel title="PortAudio 设备" status={portAudioStatus} onChange={onPortAudioChange} onApply={onPortAudioApply} onTestTone={onPortAudioTestTone} />

      <section className="panel-section feature-entry-section" aria-labelledby="feature-entry-title">
        <div className="section-heading compact-heading">
          <h2 id="feature-entry-title">话术功能</h2>
        </div>
        <div className="feature-entry-grid">
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
                ) : step.state === "error" ? (
                  <CloseCircleOutlined />
                ) : (
                  <ClockCircleOutlined />
                )}
              </span>
              <div><strong>{step.label}</strong><small>{step.detail}</small></div>
            </li>
          ))}
        </ol>
      </section>

      <section className="panel-section local-status-section" aria-labelledby="local-status-title">
        <div className="section-heading compact-heading">
          <h2 id="local-status-title">本地运行状态</h2>
          <Tag color="success">本机</Tag>
        </div>
        <dl className="local-status-list">
          {localStatus.map((item) => (
            <div key={item.label} className="local-status-item">
              <dt>{item.label}</dt>
              <dd className={item.tone ? `status-${item.tone}` : undefined}>{item.value}</dd>
            </div>
          ))}
        </dl>
      </section>
    </aside>
  );
}
