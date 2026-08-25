import { AudioOutlined, MessageOutlined, ReloadOutlined, SettingOutlined, SoundOutlined } from "@ant-design/icons";
import { Alert, Button, Checkbox, Drawer, Form, Input, InputNumber, Select, Space, Switch, Tag } from "antd";

const PRESET_OPTIONS = Array.from({ length: 22 }, (_, index) => {
  const id = `p${String(index + 1).padStart(2, "0")}`;
  const suffix = index < 20 ? "低感知" : index === 20 ? "明显加轨与空间" : "明显变调与音色";
  return { value: id, label: `${id} · ${suffix}` };
});

function DrawerFooter({ label, disabled, onClose, onSave }) {
  return (
    <Space className="drawer-footer-actions">
      <Button onClick={onClose}>取消</Button>
      <Button type="primary" disabled={disabled} onClick={onSave}>{label}</Button>
    </Space>
  );
}

function AdvancedAudioContent({ settings, onChange, onReset }) {
  return (
    <Form layout="vertical" className="feature-form advanced-audio-form">
      <Alert type="info" showIcon message="普通声音与视频处理彼此独立；参数应用失败时继续播放源音轨。" />
      <div className="drawer-field-grid">
        <Form.Item label="Host API">
          <Select value={settings.hostApi} options={[{ value: "WASAPI", label: "WASAPI（系统默认）" }, { value: "ASIO", label: "ASIO（当前不可用）", disabled: true }]} onChange={(value) => onChange?.("hostApi", value)} />
        </Form.Item>
        <Form.Item label="输出设备">
          <Select value={settings.device} options={[{ value: "default", label: "系统默认输出设备" }, { value: "speakers", label: "扬声器（Realtek Audio）" }]} onChange={(value) => onChange?.("device", value)} />
        </Form.Item>
      </div>
      <Form.Item label="内存缓冲（128–2048 KiB）">
        <InputNumber min={128} max={2048} step={128} value={settings.bufferKib} addonAfter="KiB" onChange={(value) => onChange?.("bufferKib", value)} />
      </Form.Item>
      <Form.Item label="随机多轨合一">
        <Switch checked={settings.multiTrack} onChange={(value) => onChange?.("multiTrack", value)} />
      </Form.Item>
      <div className="drawer-field-grid">
        <Form.Item label="最少随机轨数">
          <InputNumber min={1} max={4} value={settings.minTracks} disabled={!settings.multiTrack} onChange={(value) => onChange?.("minTracks", value)} />
        </Form.Item>
        <Form.Item label="最多随机轨数">
          <InputNumber min={1} max={4} value={settings.maxTracks} disabled={!settings.multiTrack} onChange={(value) => onChange?.("maxTracks", value)} />
        </Form.Item>
      </div>
      <Form.Item label="声音参数值预设" extra="p01–p20 为默认低感知池；p21、p22 只在显式选择后参与。">
        <Select mode="multiple" maxTagCount="responsive" value={settings.presetIds} options={PRESET_OPTIONS} onChange={(value) => onChange?.("presetIds", value)} />
      </Form.Item>
      <Alert type="success" showIcon message="当前只读参数" description="音高 -0.018 半音 · 播放速度 0.9988 倍 · 共振峰偏移 0.105% · 目标 SNR 自动" />
      <Space wrap>
        <Button icon={<SoundOutlined />}>播放 440 Hz 测试音</Button>
        <Button icon={<ReloadOutlined />} onClick={onReset}>恢复默认</Button>
      </Space>
    </Form>
  );
}

function InterruptionContent({ settings, onChange, onPreview }) {
  const invalidRange = !Number.isFinite(settings.minSeconds) || !Number.isFinite(settings.maxSeconds) || settings.minSeconds > settings.maxSeconds;
  return (
    <Form layout="vertical" className="feature-form interruption-form">
      <Alert type="info" showIcon message="插话会递归读取本地音频目录，并在播放期间自动压低当前源音轨。" />
      <Form.Item label="启用随机插话"><Switch checked={settings.enabled} onChange={(value) => onChange?.("enabled", value)} /></Form.Item>
      <Form.Item label="插话音频目录" extra="原型只演示配置；桌面正式版使用受控文件夹选择器。">
        <Space.Compact block>
          <Input value={settings.directory} readOnly placeholder="尚未选择本地音频目录" />
          <Button onClick={() => onChange?.("directory", "D:/Audio/Interludes")}>选择</Button>
        </Space.Compact>
      </Form.Item>
      <Form.Item label="音轨选择方式">
        <Select value={settings.selectionMode} options={[{ value: "random", label: "从所选预设随机" }, { value: "fixed", label: "固定声音预设" }]} onChange={(value) => onChange?.("selectionMode", value)} />
      </Form.Item>
      <Form.Item label="插话声音预设">
        <Select mode={settings.selectionMode === "random" ? "multiple" : undefined} maxTagCount="responsive" value={settings.selectionMode === "random" ? settings.presetIds : settings.fixedPresetId} options={PRESET_OPTIONS} onChange={(value) => onChange?.(settings.selectionMode === "random" ? "presetIds" : "fixedPresetId", value)} />
      </Form.Item>
      <Form.Item label="随机触发间隔" validateStatus={invalidRange ? "error" : undefined} help={invalidRange ? "最小间隔不能大于最大间隔" : "允许范围 0.5–60 秒；默认 8–13 秒"}>
        <Space.Compact block>
          <InputNumber aria-label="随机插话最小间隔" min={0.5} max={60} step={0.5} value={settings.minSeconds} addonAfter="秒" onChange={(value) => onChange?.("minSeconds", value)} />
          <InputNumber aria-label="随机插话最大间隔" min={0.5} max={60} step={0.5} value={settings.maxSeconds} addonAfter="秒" onChange={(value) => onChange?.("maxSeconds", value)} />
        </Space.Compact>
      </Form.Item>
      <div className="drawer-field-grid">
        <Form.Item label="插话音量"><InputNumber min={-60} max={12} value={settings.volumeDb} addonAfter="dB" onChange={(value) => onChange?.("volumeDb", value)} /></Form.Item>
        <Form.Item label="原声压低"><InputNumber min={-60} max={0} value={settings.duckingDb} addonAfter="dB" onChange={(value) => onChange?.("duckingDb", value)} /></Form.Item>
        <Form.Item label="压低过渡"><InputNumber min={0} max={1000} value={settings.attackMs} addonAfter="ms" onChange={(value) => onChange?.("attackMs", value)} /></Form.Item>
        <Form.Item label="恢复过渡"><InputNumber min={0} max={3000} value={settings.releaseMs} addonAfter="ms" onChange={(value) => onChange?.("releaseMs", value)} /></Form.Item>
      </div>
      <Button icon={<SoundOutlined />} disabled={!settings.directory} onClick={onPreview}>试听一次插话与压低包络</Button>
    </Form>
  );
}

function FixedSpeechContent({ settings, onChange, onPreview }) {
  const textLength = settings.text?.trim().length || 0;
  const invalid = textLength < 1 || textLength > 500;
  return (
    <Form layout="vertical" className="feature-form fixed-speech-form">
      <Alert type="success" showIcon message="仅调用本机系统语音，不下载模型、不识别人声。" description="朗读时暂停插话并临时静音原声，完成后自动恢复。" />
      <Form.Item label="预制文案"><Select value="welcome" options={[{ value: "welcome", label: "开场欢迎" }, { value: "follow", label: "关注提醒" }]} /></Form.Item>
      <Form.Item label="预制标题"><Input value={settings.title} maxLength={80} showCount onChange={(event) => onChange?.("title", event.target.value)} /></Form.Item>
      <Form.Item required label="朗读正文" validateStatus={invalid ? "error" : "success"} help={invalid ? "请输入 1–500 个字符" : "正文长度有效"}>
        <Input.TextArea value={settings.text} maxLength={500} showCount autoSize={{ minRows: 7, maxRows: 12 }} onChange={(event) => onChange?.("text", event.target.value)} />
      </Form.Item>
      <Space wrap><Tag>本机系统默认语音</Tag><Checkbox defaultChecked>朗读时暂停随机插话</Checkbox></Space>
      <Button icon={<SoundOutlined />} disabled={invalid} onClick={onPreview}>播放当前文案</Button>
    </Form>
  );
}

export function FeatureDrawers({ activeDrawer, advancedAudio, interruption, fixedSpeech, onClose, onAdvancedAudioChange, onAdvancedAudioReset, onInterruptionChange, onInterruptionPreview, onFixedSpeechChange, onFixedSpeechPreview, onSave }) {
  const interruptionInvalid = !Number.isFinite(interruption.minSeconds) || !Number.isFinite(interruption.maxSeconds) || interruption.minSeconds > interruption.maxSeconds;
  const fixedSpeechInvalid = !fixedSpeech.text?.trim() || fixedSpeech.text.trim().length > 500;
  return (
    <>
      <Drawer className="feature-drawer feature-drawer-advancedAudio" title={<span className="drawer-title"><SettingOutlined />高级声音设置</span>} width="min(760px, 100vw)" open={activeDrawer === "advancedAudio"} onClose={onClose} footer={<DrawerFooter label="应用声音参数" onClose={onClose} onSave={() => onSave?.("advancedAudio")} />}>
        <AdvancedAudioContent settings={advancedAudio} onChange={onAdvancedAudioChange} onReset={onAdvancedAudioReset} />
      </Drawer>
      <Drawer className="feature-drawer feature-drawer-interruption" title={<span className="drawer-title"><AudioOutlined />随机插话</span>} width="min(560px, 100vw)" open={activeDrawer === "interruption"} onClose={onClose} footer={<DrawerFooter label="保存插话配置" disabled={interruptionInvalid} onClose={onClose} onSave={() => onSave?.("interruption")} />}>
        <InterruptionContent settings={interruption} onChange={onInterruptionChange} onPreview={onInterruptionPreview} />
      </Drawer>
      <Drawer className="feature-drawer feature-drawer-fixedSpeech" title={<span className="drawer-title"><MessageOutlined />固定话术</span>} width="min(560px, 100vw)" open={activeDrawer === "fixedSpeech"} onClose={onClose} footer={<DrawerFooter label="保存本地文案" disabled={fixedSpeechInvalid} onClose={onClose} onSave={() => onSave?.("fixedSpeech")} />}>
        <FixedSpeechContent settings={fixedSpeech} onChange={onFixedSpeechChange} onPreview={onFixedSpeechPreview} />
      </Drawer>
    </>
  );
}
