import { AudioOutlined } from "@ant-design/icons";
import { Button, Form, InputNumber, Select, Space, Tag } from "antd";

const HOST_API_OPTIONS = [
  { value: "all", label: "全部 Host API" },
  { value: "wasapi", label: "WASAPI" },
  { value: "mme", label: "MME" },
  { value: "dsound", label: "DirectSound" },
  { value: "wdmks", label: "WDMKS" },
  { value: "asio", label: "ASIO（未检测到设备）", disabled: true },
];

const OUTPUT_DEVICE_OPTIONS = [
  { value: "default", label: "系统默认输出设备", hostApi: "all" },
  { value: "wasapi-speakers", label: "扬声器 · Realtek Audio", hostApi: "wasapi" },
  { value: "mme-speakers", label: "扬声器 · MME", hostApi: "mme" },
  { value: "dsound-speakers", label: "扬声器 · DirectSound", hostApi: "dsound" },
];

export function PortAudioDevicePanel({ title = "PortAudio 设备", status = {}, onChange, onApply, onTestTone }) {
  const active = status.processingStatus === "运行中";
  const devices = OUTPUT_DEVICE_OPTIONS.filter(({ hostApi }) => (
    status.hostApi === "all" || hostApi === status.hostApi
  ));
  const invalidBuffer = !Number.isFinite(status.memoryBufferKib) || status.memoryBufferKib < 128 || status.memoryBufferKib > 2048;
  const invalidDevice = !devices.some(({ value }) => value === status.outputDeviceId);
  const appliedHostApi = HOST_API_OPTIONS.find(({ value }) => value === status.appliedHostApi)?.label ?? "—";
  const appliedDevice = OUTPUT_DEVICE_OPTIONS.find(({ value }) => value === status.appliedOutputDeviceId)?.label ?? "—";
  return (
    <section className="panel-section portaudio-device-section" aria-labelledby="portaudio-device-title">
      <div className="section-heading compact-heading">
        <h2 id="portaudio-device-title">{title}</h2>
        <Tag color={active ? "success" : "default"} icon={<AudioOutlined />}>
          {active ? "已接管" : "已旁路"}
        </Tag>
      </div>
      <Form className="portaudio-device-form" layout="vertical">
        <Form.Item label="Host API">
          <Select aria-label="PortAudio Host API" size="small" value={status.hostApi} options={HOST_API_OPTIONS} onChange={(value) => onChange?.("hostApi", value)} style={{ width: "100%" }} />
        </Form.Item>
        <Form.Item label="输出设备">
          <Select aria-label="PortAudio 输出设备" size="small" value={status.outputDeviceId} options={devices} onChange={(value) => onChange?.("outputDeviceId", value)} style={{ width: "100%" }} />
        </Form.Item>
        <Form.Item label="内存缓冲" extra="128–2048 KiB" validateStatus={invalidBuffer ? "error" : undefined} help={invalidBuffer ? "请输入 128–2048 KiB" : undefined}>
          <InputNumber aria-label="PortAudio 内存缓冲" size="small" min={128} max={2048} step={128} value={status.memoryBufferKib} addonAfter="KiB" onChange={(value) => onChange?.("memoryBufferKib", value)} style={{ width: "100%" }} />
        </Form.Item>
      </Form>
      <dl className="portaudio-status-list">
        <div className="portaudio-status-item"><dt>实际出口</dt><dd>{status.actualOutput ?? "—"}</dd></div>
        <div className="portaudio-status-item"><dt>已应用配置</dt><dd title={`${appliedHostApi} · ${appliedDevice} · ${status.appliedMemoryBufferKib} KiB`}>{appliedHostApi} · {appliedDevice} · {status.appliedMemoryBufferKib} KiB</dd></div>
        <div className="portaudio-status-item"><dt>处理状态</dt><dd>{status.dirty ? "待应用" : status.processingStatus ?? "—"}</dd></div>
      </dl>
      <Space.Compact className="portaudio-device-actions" block>
        <Button type="primary" disabled={!status.dirty || invalidBuffer || invalidDevice} onClick={onApply}>应用设置</Button>
        <Button disabled={!active} onClick={onTestTone}>播放测试音</Button>
      </Space.Compact>
    </section>
  );
}
