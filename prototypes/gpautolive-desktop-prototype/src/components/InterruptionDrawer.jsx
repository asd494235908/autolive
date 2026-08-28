import {
  Alert,
  Button,
  Checkbox,
  Input,
  InputNumber,
  Select,
  Space,
  Switch,
  Tag,
  Typography,
} from "antd";
import { AUDIO_PRESET_OPTIONS, DEFAULT_AUDIO_PRESET_IDS } from "../data/audio-preset-options.js";
import { FeatureDrawerSection, FeatureDrawerShell } from "./FeatureDrawerShell.jsx";

const DEFAULT_SETTINGS = {
  enabled: false,
  directory: null,
  audioSelectionMode: "random",
  audioFixedPresetId: "p01",
  audioPresetIds: DEFAULT_AUDIO_PRESET_IDS,
  audioMixEnabled: false,
  audioMixPickMin: 1,
  audioMixPickMax: 2,
  audioVariationMode: "each_playback",
  audioVariationPeriodMinMs: 8_000,
  audioVariationPeriodMaxMs: 15_000,
  intervalMinMs: 8_000,
  intervalMaxMs: 13_000,
  volumeDb: 0,
  duckingDepthDb: -12,
  duckingAttackMs: 50,
  duckingReleaseMs: 250,
};

function inRange(value, min, max) {
  return Number.isFinite(value) && value >= min && value <= max;
}

function seconds(milliseconds) {
  return Number.isFinite(milliseconds) ? milliseconds / 1_000 : null;
}

function SettingField({ label, htmlFor, hint, children }) {
  return (
    <div className="interruption-drawer__field">
      <label htmlFor={htmlFor}>{label}</label>
      {children}
      {hint ? <Typography.Text>{hint}</Typography.Text> : null}
    </div>
  );
}

function NumberField({ label, ariaLabel, value, unit, onChange, ...limits }) {
  return (
    <div className="interruption-drawer__number-field">
      {label ? <Typography.Text className="interruption-drawer__number-label">{label}</Typography.Text> : null}
      <InputNumber
        {...limits}
        aria-label={ariaLabel}
        value={value}
        onChange={(nextValue) => typeof nextValue === "number" && onChange(nextValue)}
        style={{ width: "100%" }}
      />
      {unit ? <Typography.Text className="interruption-drawer__number-unit">{unit}</Typography.Text> : null}
    </div>
  );
}

export function InterruptionDrawer({
  settings,
  onChange,
  onClose,
  onSave,
  open,
}) {
  const value = { ...DEFAULT_SETTINGS, ...settings };
  const selectedPresetIds = Array.isArray(value.audioPresetIds) ? value.audioPresetIds : [];
  const fixedMode = value.audioSelectionMode === "fixed";
  const periodicMode = value.audioVariationMode === "periodic";
  const validationErrors = [];

  if (value.enabled && !value.directory) validationErrors.push("启用随机插话前必须选择音频目录");
  if (!fixedMode && selectedPresetIds.length === 0) validationErrors.push("随机音轨至少保留一个声音预设");
  if (!fixedMode && value.audioMixEnabled && (
    !inRange(value.audioMixPickMin, 1, 4)
    || !inRange(value.audioMixPickMax, 1, 4)
    || value.audioMixPickMin > value.audioMixPickMax
  )) validationErrors.push("随机多轨范围必须为 1–4，且最少轨数不能大于最多轨数");
  if (!fixedMode && periodicMode && (
    !inRange(value.audioVariationPeriodMinMs, 1_000, 60_000)
    || !inRange(value.audioVariationPeriodMaxMs, 1_000, 60_000)
    || value.audioVariationPeriodMinMs > value.audioVariationPeriodMaxMs
  )) validationErrors.push("变化周期必须为 1–60 秒，且最小值不能大于最大值");
  if (
    !inRange(value.intervalMinMs, 500, 60_000)
    || !inRange(value.intervalMaxMs, 500, 60_000)
    || value.intervalMinMs > value.intervalMaxMs
  ) validationErrors.push("触发间隔必须为 0.5–60 秒，且最小值不能大于最大值");
  if (!inRange(value.volumeDb, -60, 12)) validationErrors.push("插话音量必须为 -60–12 dB");
  if (!inRange(value.duckingDepthDb, -60, 0)) validationErrors.push("原声压低必须为 -60–0 dB");
  if (!inRange(value.duckingAttackMs, 0, 1_000)) validationErrors.push("压低过渡必须为 0–1000 ms");
  if (!inRange(value.duckingReleaseMs, 0, 3_000)) validationErrors.push("恢复过渡必须为 0–3000 ms");

  const update = (field, nextValue) => onChange?.(field, nextValue);
  const updateMixMaximum = (nextValue) => {
    update("audioMixPickMax", nextValue);
    if (value.audioMixPickMin > nextValue) update("audioMixPickMin", nextValue);
  };

  return (
    <FeatureDrawerShell
      modifierClass="interruption-drawer"
      title="随机插话"
      description="递归扫描本地音频目录及其子目录，并在插话期间自动压低视频原声。"
      width="min(560px, 100vw)"
      open={open}
      onClose={onClose}
      summaryLabel="随机插话状态概览"
      summary={(
        <>
          <Tag color={value.enabled ? "success" : "default"}>{value.enabled ? "已启用" : "未启用"}</Tag>
          <Tag color={value.dirty ? "warning" : "blue"}>{value.dirty ? "有未保存更改" : "配置已同步"}</Tag>
          <Tag>{value.directory ? "音频目录已选择" : "未选择音频目录"}</Tag>
          <Tag>{fixedMode ? `固定 ${value.audioFixedPresetId}` : value.audioMixEnabled ? `随机合一 ${value.audioMixPickMin}–${value.audioMixPickMax} 轨` : "随机单轨"}</Tag>
          {!fixedMode ? <Tag>{periodicMode ? "周期换组" : "每次插话换组"}</Tag> : null}
        </>
      )}
      footer={(
        <Button type="primary" disabled={validationErrors.length > 0} onClick={onSave}>
          保存插话配置
        </Button>
      )}
    >
        {validationErrors.length > 0 ? (
          <Alert
            className="interruption-drawer__validation"
            type="warning"
            showIcon
            message="请检查插话配置"
            description={validationErrors.join("；")}
          />
        ) : null}

        <FeatureDrawerSection
          title="启用状态"
          description="关闭后保留当前设置，但不会在播放过程中触发插话。"
          extra={<Switch aria-label="启用随机插话" checked={value.enabled} onChange={(checked) => update("enabled", checked)} />}
        >
          <Typography.Text className="interruption-drawer__muted">插话按独立随机周期运行，不会改变视频循环进度。</Typography.Text>
        </FeatureDrawerSection>

        <FeatureDrawerSection title="音频来源" description="选择插话音频根目录，系统会递归扫描其子目录。">
          <SettingField label="插话音频目录" htmlFor="prototype-interruption-directory">
            <Space.Compact style={{ width: "100%" }}>
              <Input id="prototype-interruption-directory" readOnly value={value.directory ?? ""} placeholder="请选择音频文件夹" />
              <Button onClick={() => update("directory", "D:/Audio/Interludes")}>选择音频文件夹</Button>
            </Space.Compact>
          </SettingField>
        </FeatureDrawerSection>

        <FeatureDrawerSection title="独立声音轨" description="插话可固定一条预设，或从自己的 22 项预设池随机抽样；配置变化从下一段插话开始生效。">
          <SettingField label="音轨选择方式" htmlFor="prototype-interruption-selection-mode">
            <Select
              id="prototype-interruption-selection-mode"
              aria-label="插话音轨选择方式"
              value={value.audioSelectionMode}
              options={[
                { value: "fixed", label: "固定选择" },
                { value: "random", label: "从所选音轨随机" },
              ]}
              onChange={(nextValue) => update("audioSelectionMode", nextValue)}
              style={{ width: "100%" }}
            />
          </SettingField>

          {fixedMode ? (
            <SettingField label="固定声音预设" htmlFor="prototype-interruption-fixed-preset">
              <Select
                id="prototype-interruption-fixed-preset"
                aria-label="插话固定声音预设"
                value={value.audioFixedPresetId}
                options={AUDIO_PRESET_OPTIONS}
                onChange={(nextValue) => update("audioFixedPresetId", nextValue)}
                style={{ width: "100%" }}
              />
            </SettingField>
          ) : (
            <>
              <div className="interruption-drawer__toggle-row">
                <div>
                  <strong>随机多轨合一</strong>
                  <Typography.Text>关闭时每段只抽一轨；开启后随机抽取 1–4 轨等权合一。</Typography.Text>
                </div>
                <Switch aria-label="随机多轨合一" checked={value.audioMixEnabled} onChange={(checked) => update("audioMixEnabled", checked)} />
              </div>

              {value.audioMixEnabled ? (
                <div className="interruption-drawer__field-grid interruption-drawer__field-grid--compact">
                  <SettingField label="最少随机轨数">
                    <InputNumber aria-label="插话最少随机轨数" min={1} max={4} value={value.audioMixPickMin} onChange={(nextValue) => typeof nextValue === "number" && update("audioMixPickMin", Math.min(nextValue, value.audioMixPickMax))} style={{ width: "100%" }} />
                  </SettingField>
                  <SettingField label="最多随机轨数">
                    <InputNumber aria-label="插话最多随机轨数" min={1} max={4} value={value.audioMixPickMax} onChange={(nextValue) => typeof nextValue === "number" && updateMixMaximum(nextValue)} style={{ width: "100%" }} />
                  </SettingField>
                </div>
              ) : null}

              <SettingField label="插话声音预设" hint="22 项均可进入插话随机池，至少保留一项；p21、p22 为明显强效果。">
                <Space direction="vertical" size="small" style={{ width: "100%" }}>
                  <Button size="small" aria-label="全选插话声音预设" onClick={() => update("audioPresetIds", AUDIO_PRESET_OPTIONS.map(({ value: id }) => id))}>全选</Button>
                  <Checkbox.Group
                    aria-label="插话声音预设"
                    value={selectedPresetIds}
                    onChange={(nextValues) => nextValues.length > 0 && update("audioPresetIds", nextValues.map(String))}
                    style={{ width: "100%" }}
                  >
                    <div className="interruption-drawer__checkbox-grid">
                      {AUDIO_PRESET_OPTIONS.map((option) => <Checkbox key={option.value} value={option.value}>{option.label}</Checkbox>)}
                    </div>
                  </Checkbox.Group>
                </Space>
              </SettingField>

              <SettingField label="抽样方式" htmlFor="prototype-interruption-variation-mode">
                <Select
                  id="prototype-interruption-variation-mode"
                  aria-label="插话声音抽样方式"
                  value={value.audioVariationMode}
                  options={[
                    { value: "each_playback", label: "每次插话重新随机" },
                    { value: "periodic", label: "按周期更新" },
                  ]}
                  onChange={(nextValue) => update("audioVariationMode", nextValue)}
                  style={{ width: "100%" }}
                />
              </SettingField>

              {periodicMode ? (
                <div className="interruption-drawer__field-grid interruption-drawer__field-grid--compact">
                  <SettingField label="变化周期最小值">
                    <NumberField ariaLabel="插话变化周期最小值（秒）" unit="秒" min={1} max={60} value={seconds(value.audioVariationPeriodMinMs)} onChange={(nextValue) => update("audioVariationPeriodMinMs", Math.round(nextValue * 1_000))} />
                  </SettingField>
                  <SettingField label="变化周期最大值">
                    <NumberField ariaLabel="插话变化周期最大值（秒）" unit="秒" min={1} max={60} value={seconds(value.audioVariationPeriodMaxMs)} onChange={(nextValue) => update("audioVariationPeriodMaxMs", Math.round(nextValue * 1_000))} />
                  </SettingField>
                </div>
              ) : null}
            </>
          )}

          <Alert
            type="info"
            showIcon
            message={`当前由 ${value.actualAudioOutputLabel ?? "PortAudio"} 应用插话声音预设。`}
            description="PortAudio 与 WebView 均应用固定或随机预设与多轨合一；WebView 本地处理失败时明确回退插话原声。周期到期只影响下一段，不会重启当前插话。"
          />
        </FeatureDrawerSection>

        <FeatureDrawerSection title="触发与混音" description="间隔决定插话频率；音量与包络决定原声压低和恢复速度。">
          <div className="interruption-drawer__parameter-groups">
            <div className="interruption-drawer__parameter-group">
              <strong>触发间隔</strong>
              <div className="interruption-drawer__field-grid">
                <NumberField label="最小间隔" ariaLabel="插话最小间隔（秒）" unit="秒" min={0.5} max={60} step={0.5} value={seconds(value.intervalMinMs)} onChange={(nextValue) => update("intervalMinMs", Math.round(nextValue * 1_000))} />
                <NumberField label="最大间隔" ariaLabel="插话最大间隔（秒）" unit="秒" min={0.5} max={60} step={0.5} value={seconds(value.intervalMaxMs)} onChange={(nextValue) => update("intervalMaxMs", Math.round(nextValue * 1_000))} />
              </div>
            </div>
            <div className="interruption-drawer__parameter-group">
              <strong>音量与原声压低</strong>
              <div className="interruption-drawer__field-grid">
                <NumberField label="插话音量" ariaLabel="插话音量" unit="dB" min={-60} max={12} step={0.5} value={value.volumeDb} onChange={(nextValue) => update("volumeDb", nextValue)} />
                <NumberField label="原声压低" ariaLabel="原声压低" unit="dB" min={-60} max={0} step={0.5} value={value.duckingDepthDb} onChange={(nextValue) => update("duckingDepthDb", nextValue)} />
              </div>
            </div>
            <div className="interruption-drawer__parameter-group">
              <strong>过渡时间</strong>
              <div className="interruption-drawer__field-grid">
                <NumberField label="原声压低过渡" ariaLabel="原声压低过渡" unit="ms" min={0} max={1_000} value={value.duckingAttackMs} onChange={(nextValue) => update("duckingAttackMs", nextValue)} />
                <NumberField label="原声恢复过渡" ariaLabel="原声恢复过渡" unit="ms" min={0} max={3_000} value={value.duckingReleaseMs} onChange={(nextValue) => update("duckingReleaseMs", nextValue)} />
              </div>
            </div>
          </div>
        </FeatureDrawerSection>
    </FeatureDrawerShell>
  );
}

export default InterruptionDrawer;
