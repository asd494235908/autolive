import {
  ClearOutlined,
  ReloadOutlined,
} from '@ant-design/icons';
import {
  Alert,
  Button,
  Checkbox,
  Descriptions,
  Form,
  InputNumber,
  Popconfirm,
  Space,
  Switch,
  Tag,
  Typography,
} from 'antd';
import { AUDIO_PRESET_OPTIONS, DEFAULT_AUDIO_PRESET_IDS } from '../data/audio-preset-options.js';
import { AUDIO_PARAMETERS } from '../data/prototype-data.js';
import { FeatureDrawerSection, FeatureDrawerShell } from './FeatureDrawerShell.jsx';

const STATUS_META = {
  active: { color: 'success', label: '已生效' },
  processing: { color: 'processing', label: '处理中' },
  pending: { color: 'warning', label: '待应用' },
  readonly: { color: 'default', label: '只读诊断' },
  bypassed: { color: 'default', label: '已旁路' },
};

function StatusTag({ status }) {
  const meta = STATUS_META[status] ?? STATUS_META.active;
  return <Tag color={meta.color}>{meta.label}</Tag>;
}

export function AdvancedAudioDrawer({
  settings = {},
  processingEnabled = true,
  onProcessingEnabledChange,
  onChange,
  onRegenerate,
  onClose,
  onSave,
  onCleanup,
  open = false,
}) {
  const multiTrack = settings.mixEnabled ?? false;
  const minTracks = settings.mixPickMin ?? 1;
  const maxTracks = settings.mixPickMax ?? 2;
  const presetIds = settings.selectedPresetIds ?? DEFAULT_AUDIO_PRESET_IDS;
  const invalidTracks = minTracks > maxTracks;
  const invalidSettings = invalidTracks || presetIds.length === 0;

  const change = (field, value) => onChange?.(field, value);

  return (
    <FeatureDrawerShell
      modifierClass="advanced-audio-drawer"
      title="高级声音设置"
      description="管理普通声音处理、多轨预设和参数生效状态。"
      width="min(760px, 100vw)"
      open={open}
      onClose={onClose}
      summaryLabel="高级声音状态概览"
      summary={(
        <>
          <Tag color={processingEnabled ? 'success' : 'default'}>{processingEnabled ? '声音处理已开启' : '声音处理未开启'}</Tag>
          <Tag color={processingEnabled ? 'processing' : 'default'}>{processingEnabled ? '实时值' : '未启用'}</Tag>
          <Tag>实际输出：{settings.actualOutputLabel ?? 'PortAudio'}</Tag>
          <Tag>实际混音：{settings.actualMixLabel ?? (multiTrack ? `${minTracks}–${maxTracks} 条支路` : '1 条支路')}</Tag>
        </>
      )}
      footer={(
        <>
          <Button icon={<ReloadOutlined />} onClick={onRegenerate}>
            重新生成本周期参数
          </Button>
          <Button type="primary" disabled={invalidSettings || !processingEnabled} onClick={() => onSave?.(settings)}>
            应用声音参数
          </Button>
        </>
      )}
    >
        <FeatureDrawerSection
          title="处理与输出"
          description="普通声音与视频处理开关彼此独立；这里只应用声音参数。"
          extra={(
            <Switch
              aria-label="高级声音处理"
              checked={processingEnabled}
              onChange={onProcessingEnabledChange}
            />
          )}
        >
          <div className="advanced-audio-drawer__status-grid">
            <div><Typography.Text type="secondary">处理状态</Typography.Text><strong>{processingEnabled ? '处理中' : '已关闭'}</strong></div>
            <div><Typography.Text type="secondary">当前出口</Typography.Text><strong>{settings.actualOutputLabel ?? 'PortAudio'}</strong></div>
            <div><Typography.Text type="secondary">混音支路</Typography.Text><strong>{settings.actualMixLabel ?? (multiTrack ? `${minTracks}–${maxTracks} 条支路` : '1 条支路')}</strong></div>
          </div>
        </FeatureDrawerSection>

        <FeatureDrawerSection
          title="多轨与预设"
          description="从已勾选预设中抽样；p01–p20 为默认低感知池，p21、p22 为手动明显效果。"
          extra={<Tag color={presetIds.length ? 'blue' : 'error'}>已选 {presetIds.length} 项</Tag>}
        >
          <div className="advanced-audio-drawer__toggle-row">
            <div><strong>多轨合并</strong><Typography.Text type="secondary">将多套声音预设合并为当前输出。</Typography.Text></div>
            <Switch aria-label="多轨合并" checked={multiTrack} disabled={!processingEnabled} onChange={(checked) => change('mixEnabled', checked)} />
          </div>
          {multiTrack ? (
            <Form className="advanced-audio-drawer__form" layout="vertical">
              <div className="advanced-audio-drawer__field-grid drawer-field-grid">
                <Form.Item label="最少随机轨数">
                  <InputNumber aria-label="最少随机轨数" min={1} max={4} value={minTracks} onChange={(value) => change('mixPickMin', value)} style={{ width: '100%' }} />
                </Form.Item>
                <Form.Item label="最多随机轨数" validateStatus={invalidTracks ? 'error' : undefined} help={invalidTracks ? '不能小于最少随机轨数' : undefined}>
                  <InputNumber aria-label="最多随机轨数" min={1} max={4} value={maxTracks} onChange={(value) => change('mixPickMax', value)} style={{ width: '100%' }} />
                </Form.Item>
              </div>
            </Form>
          ) : null}
          <div className="advanced-audio-drawer__preset-actions">
            <Space wrap>
              <Button size="small" onClick={() => change('selectedPresetIds', DEFAULT_AUDIO_PRESET_IDS)}>选择默认 20 项</Button>
              <Button size="small" onClick={() => change('selectedPresetIds', AUDIO_PRESET_OPTIONS.map(({ value }) => value))}>全选 22 项</Button>
            </Space>
          </div>
          <Checkbox.Group
            className="advanced-audio-drawer__preset-grid"
            aria-label="声音参数值预设"
            value={presetIds}
            disabled={!processingEnabled}
            options={AUDIO_PRESET_OPTIONS}
            onChange={(values) => values.length && change('selectedPresetIds', values.map(String))}
          />
          {!presetIds.length ? <Alert type="warning" showIcon message="至少选择一项声音预设" /> : null}
        </FeatureDrawerSection>

        <FeatureDrawerSection title="音频参数状态" description="展示正式普通声音参数的当前值及原型处理状态。">
          <Descriptions className="advanced-audio-drawer__parameter-status" column={{ xs: 1, sm: 1, md: 2 }} size="small" bordered>
            {AUDIO_PARAMETERS.map((parameter) => {
              const status = settings.parameterStatuses?.[parameter.key]
                ?? (parameter.key === 'current_formant_hz' ? 'readonly' : processingEnabled ? 'active' : 'bypassed');
              return (
                <Descriptions.Item key={parameter.key} label={parameter.label}>
                  <Space size={4} wrap>
                    <span>{settings.parameterValues?.[parameter.key] ?? parameter.value}</span>
                    <StatusTag status={status} />
                  </Space>
                </Descriptions.Item>
              );
            })}
          </Descriptions>
        </FeatureDrawerSection>

        <FeatureDrawerSection title="缓存管理" description="只删除未被当前播放或待切换任务引用的处理缓存。">
          <div className="advanced-audio-drawer__cache-row" aria-live="polite">
            <Popconfirm
              title="删除已生成缓存？"
              description="只删除未被当前或待切换引用的处理缓存。"
              onConfirm={onCleanup}
            >
              <Button icon={<ClearOutlined />} loading={settings.cleanupBusy}>删除已生成缓存</Button>
            </Popconfirm>
            {settings.cleanupSummary ? <Tag>{settings.cleanupSummary}</Tag> : null}
          </div>
          {settings.cleanupError ? <Alert type="error" showIcon message={settings.cleanupError} /> : null}
        </FeatureDrawerSection>
    </FeatureDrawerShell>
  );
}

export default AdvancedAudioDrawer;
