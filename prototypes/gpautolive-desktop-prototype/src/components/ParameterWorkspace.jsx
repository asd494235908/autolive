import {
  AudioOutlined,
  ReloadOutlined,
  SoundOutlined,
  VideoCameraOutlined,
} from '@ant-design/icons';
import { Button, Card, InputNumber, Progress, Space, Switch, Tag, Tooltip, Typography } from 'antd';
import {
  ADVANCED_PARAMETERS,
  AUDIO_PARAMETERS,
  DEFAULT_CYCLE_STATES,
  PARAMETER_PALETTE,
  VIDEO_PARAMETERS,
} from '../data/prototype-data.js';
import { getInterruptionPresetChanges } from '../data/interruption-preset-changes.js';
import { AudioProcessingPanel } from './AudioProcessingPanel.jsx';
import { InlineRecoveryAlert } from './InlineRecoveryAlert.jsx';

const { Text, Title } = Typography;

function clampProgress(value) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.min(100, Math.max(0, parsed)) : 0;
}

function formatSeconds(milliseconds) {
  const seconds = Number(milliseconds) / 1000;
  return Number.isInteger(seconds) ? String(seconds) : seconds.toFixed(1);
}

function ParameterRow({ parameter, colorIndex }) {
  const accent = PARAMETER_PALETTE[colorIndex % PARAMETER_PALETTE.length];

  return (
    <div
      className="parameter-row"
      style={{ '--parameter-accent': accent }}
    >
      <div className="parameter-card__summary">
        <Text className="parameter-card__label" title={parameter.label}>{parameter.label}</Text>
        <Text className="parameter-card__value" strong title={parameter.value}>{parameter.value}</Text>
      </div>
    </div>
  );
}

function ParameterSection({ title, parameters, startColorIndex = 0, summary }) {
  return (
    <section className="parameter-section" aria-labelledby={`parameter-section-${title}`}>
      <div className="parameter-section__heading">
        <Title id={`parameter-section-${title}`} level={5}>{title}</Title>
        <Text type="secondary">{summary ?? `${parameters.length} 项 · 只读快照`}</Text>
      </div>
      <div className="parameter-section__grid">
        {parameters.map((parameter, index) => (
          <ParameterRow
            key={parameter.key}
            parameter={parameter}
            colorIndex={startColorIndex + index}
          />
        ))}
      </div>
    </section>
  );
}

function CycleRange({ title, minValue, maxValue, onChange }) {
  return (
    <div className="cycle-range-row">
      <Text type="secondary">范围</Text>
      <InputNumber
        aria-label={`${title}最小秒数`}
        min={1}
        max={60}
        size="small"
        style={{ width: '100%' }}
        value={minValue}
        onChange={(value) => onChange?.('min', value)}
      />
      <span aria-hidden="true">—</span>
      <InputNumber
        aria-label={`${title}最大秒数`}
        min={1}
        max={60}
        size="small"
        style={{ width: '100%' }}
        value={maxValue}
        onChange={(value) => onChange?.('max', value)}
      />
      <Text>秒</Text>
    </div>
  );
}

function CycleStatusCard({
  title,
  icon,
  enabled,
  isPlaying,
  cycle,
  colorIndex,
  rangeLabel = '周期范围',
  minValue,
  maxValue,
  onRangeChange,
  stateLabelOverride,
  stateToneOverride,
}) {
  const accent = PARAMETER_PALETTE[colorIndex % PARAMETER_PALETTE.length];
  const active = !stateLabelOverride && enabled && isPlaying && cycle.hasNextPlan;
  const stateLabel = stateLabelOverride ?? (!enabled ? '已关闭' : !isPlaying ? '播放暂停' : cycle.stateLabel);
  const stateTone = stateToneOverride ?? (active ? 'processing' : 'default');

  return (
    <section className="cycle-status-card" style={{ '--parameter-accent': accent }} aria-label={`${title}状态`}>
      <div className="cycle-status-card__heading">
        <Space size={8}>{icon}<Text strong>{title}</Text></Space>
        <Tag color={stateTone}>{stateLabel}</Tag>
      </div>
      <div className={`cycle-status-card__meta${onRangeChange ? ' cycle-status-card__meta--editable' : ''}`}>
        {onRangeChange ? (
          <CycleRange title={title} minValue={minValue} maxValue={maxValue} onChange={onRangeChange} />
        ) : <Text type="secondary">{rangeLabel} {cycle.range}</Text>}
        <Text>已变化 {cycle.changes} 次</Text>
      </div>
      <Progress
        className="cycle-status-card__progress"
        percent={active ? clampProgress(cycle.progress) : 0}
        showInfo={false}
        size={["100%", 4]}
        strokeColor={accent}
        aria-label={`${title}下一计划进度`}
      />
    </section>
  );
}

export function ParameterWorkspace({
  videoEnabled = true,
  audioEnabled = true,
  isPlaying = true,
  onToggleVideo,
  onToggleAudio,
  onResetVideo,
  cycleRange = { videoMin: 8, videoMax: 15, audioMin: 3, audioMax: 5 },
  onCycleRangeChange,
  interruption = {},
  interruptionRuntime = {},
  runtimeIssues = {},
  recoveryBusy = null,
  onRecoverIssue,
  onRecoverySecondary,
  videoCycle = DEFAULT_CYCLE_STATES.video,
  audioCycle = DEFAULT_CYCLE_STATES.audio,
  interruptionCycle = DEFAULT_CYCLE_STATES.interruption,
}) {
  const audioIssue = runtimeIssues.engine ?? runtimeIssues.audio;
  const audioRecovering = recoveryBusy === 'engine' || recoveryBusy === 'audio';
  const videoIssue = runtimeIssues.engine ?? runtimeIssues.video;
  const videoRecovering = recoveryBusy === 'engine' || recoveryBusy === 'video';
  const interruptionIssue = runtimeIssues.interruption;
  const interruptionRecovering = recoveryBusy === 'interruption';
  const interruptionReady = Boolean(interruption.enabled && interruption.directory);
  const interruptionActive = interruptionReady && isPlaying && !interruptionIssue && !interruptionRecovering;
  const interruptionCycleState = {
    ...interruptionCycle,
    range: interruption.audioSelectionMode === 'fixed'
      ? '固定预设 · 不轮换'
      : interruption.audioVariationMode === 'periodic'
        ? `${formatSeconds(interruption.audioVariationPeriodMinMs ?? 8_000)}–${formatSeconds(interruption.audioVariationPeriodMaxMs ?? 15_000)} 秒`
        : '每次插话重新随机',
    hasNextPlan: Boolean(interruption.directory),
    stateLabel: interruption.directory ? (audioEnabled ? 'PortAudio 运行中' : 'WebView 运行中') : '等待音频目录',
  };
  const interruptionStatus = !interruption.enabled
    ? '未启用'
    : interruptionRecovering
      ? '恢复中'
      : interruptionIssue
        ? '本次已跳过'
    : !interruption.directory
      ? '等待音频'
      : !isPlaying
        ? '等待播放'
        : '已就绪';
  const presetChanges = getInterruptionPresetChanges(interruptionRuntime);

  return (
    <section className="parameter-workspace" aria-label="音视频处理">
      <div className="parameter-workspace__domains">
        <div className="media-domain-column media-domain-column--audio">
          <Card className="media-domain-card media-domain-card--audio" size="small" role="region" aria-labelledby="audio-domain-title">
            <div className="media-domain-card__heading">
              <div className="media-domain-card__heading-copy">
                <Space size={8}><SoundOutlined /><Title id="audio-domain-title" level={4}>音频</Title></Space>
                <Text type="secondary">插话与普通声音参数</Text>
              </div>
              <Space size={6} className="media-domain-card__title-actions">
                <Text>普通声音处理</Text>
                <Switch size="small" checked={audioEnabled} onChange={onToggleAudio} aria-label="普通声音处理开关" />
              </Space>
            </div>

            <section className="interruption-audio-section" aria-label="插话状态与周期">
              <InlineRecoveryAlert
                issue={runtimeIssues.interruption}
                recovering={recoveryBusy === 'interruption'}
                onRecover={() => onRecoverIssue?.('interruption')}
                onSecondary={() => onRecoverySecondary?.('interruption')}
              />
              <CycleStatusCard
                title="插话声音周期"
                icon={<AudioOutlined />}
                enabled={Boolean(interruption.enabled)}
                isPlaying={isPlaying}
                cycle={interruptionCycleState}
                colorIndex={4}
                rangeLabel={interruption.audioSelectionMode === 'random' && interruption.audioVariationMode === 'periodic' ? '周期范围' : '更新方式'}
                stateLabelOverride={interruptionRecovering ? '恢复中' : interruptionIssue ? '本次已跳过' : undefined}
                stateToneOverride={interruptionRecovering ? 'processing' : interruptionIssue ? 'error' : undefined}
              />
              <AudioProcessingPanel
                ariaLabel="插话音频状态"
                enabled={Boolean(interruption.enabled)}
                active={interruptionActive}
                actualOutput={interruptionIssue ? '未输出' : audioEnabled ? 'PortAudio' : 'WebView'}
                processingStatus={interruptionStatus}
                processingTone={interruptionRecovering ? 'processing' : interruptionIssue ? 'error' : interruptionActive ? 'processing' : interruption.enabled ? 'warning' : 'default'}
                currentPreset={presetChanges.currentPreset}
                waveformCaption="实时 · 插话混音后 PCM（最终混音）"
                waveformAriaLabel={interruptionActive ? '插话混入后的最终音频诊断波形' : '当前没有可用的插话最终混音诊断波形'}
              />
            </section>
            <InlineRecoveryAlert
              issue={runtimeIssues.engine}
              recovering={recoveryBusy === 'engine'}
              onRecover={() => onRecoverIssue?.('engine')}
              onSecondary={() => onRecoverySecondary?.('engine')}
            />
            <InlineRecoveryAlert
              issue={runtimeIssues.audio}
              recovering={recoveryBusy === 'audio'}
              onRecover={() => onRecoverIssue?.('audio')}
              onSecondary={() => onRecoverySecondary?.('audio')}
            />
            <CycleStatusCard
              title="声音周期"
              icon={<SoundOutlined />}
              enabled={audioEnabled}
              isPlaying={isPlaying}
              cycle={audioCycle}
              colorIndex={3}
              minValue={cycleRange.audioMin}
              maxValue={cycleRange.audioMax}
              onRangeChange={(edge, value) => onCycleRangeChange?.(`audio${edge === 'min' ? 'Min' : 'Max'}`, value)}
              stateLabelOverride={audioRecovering ? '恢复中' : audioIssue ? (runtimeIssues.engine ? '引擎不可用' : '失败回退') : undefined}
              stateToneOverride={audioRecovering ? 'processing' : audioIssue ? (runtimeIssues.engine ? 'warning' : 'error') : undefined}
            />
            <AudioProcessingPanel
              enabled={audioEnabled}
              active={!audioIssue && !audioRecovering && audioEnabled}
              actualOutput={audioIssue ? '原声' : undefined}
              processingStatus={audioRecovering ? '恢复中' : audioIssue ? '失败回退' : undefined}
              processingTone={audioRecovering ? 'processing' : audioIssue ? (runtimeIssues.engine ? 'warning' : 'error') : undefined}
              waveformAriaLabel={audioIssue ? '声音处理失败，当前保留原声' : undefined}
            />
            <ParameterSection title="普通声音" parameters={AUDIO_PARAMETERS} startColorIndex={1} />
          </Card>

          <Card className="media-domain-card media-domain-card--interruption-preset" size="small" role="region" aria-labelledby="interruption-preset-title">
            <div className="media-domain-card__heading">
              <Space size={8}><AudioOutlined /><Title id="interruption-preset-title" level={4}>插话声音预设</Title></Space>
              <Text type="secondary">当前插话实际参数变化</Text>
            </div>
            <ParameterSection
              title="当前参数"
              parameters={presetChanges.parameters}
              summary={presetChanges.summary}
              startColorIndex={5}
            />
          </Card>

        </div>

        <div className="media-domain-column media-domain-column--video">
          <Card className="media-domain-card media-domain-card--video" size="small" role="region" aria-labelledby="video-domain-title">
            <div className="media-domain-card__heading">
              <div className="media-domain-card__heading-copy">
                <Space size={8}><VideoCameraOutlined /><Title id="video-domain-title" level={4}>画面</Title></Space>
                <Text type="secondary">视频周期与视觉参数</Text>
              </div>
              <Space size={6} className="media-domain-card__title-actions">
                <Text>视频处理</Text>
                <Switch
                  size="small"
                  checked={videoEnabled}
                  onChange={onToggleVideo}
                  aria-label="视频处理开关"
                />
                <Tooltip title="恢复视频参数默认值">
                  <Button size="small" icon={<ReloadOutlined />} onClick={onResetVideo} aria-label="恢复默认" />
                </Tooltip>
              </Space>
            </div>
            <InlineRecoveryAlert
              issue={runtimeIssues.engine}
              recovering={recoveryBusy === 'engine'}
              onRecover={() => onRecoverIssue?.('engine')}
              onSecondary={() => onRecoverySecondary?.('engine')}
            />
            <InlineRecoveryAlert
              issue={runtimeIssues.video}
              recovering={recoveryBusy === 'video'}
              onRecover={() => onRecoverIssue?.('video')}
              onSecondary={() => onRecoverySecondary?.('video')}
            />
            <CycleStatusCard
              title="视频周期"
              icon={<VideoCameraOutlined />}
              enabled={videoEnabled}
              isPlaying={isPlaying}
              cycle={videoCycle}
              colorIndex={0}
              minValue={cycleRange.videoMin}
              maxValue={cycleRange.videoMax}
              onRangeChange={(edge, value) => onCycleRangeChange?.(`video${edge === 'min' ? 'Min' : 'Max'}`, value)}
              stateLabelOverride={videoRecovering ? '恢复中' : videoIssue ? (runtimeIssues.engine ? '引擎不可用' : '失败回退') : undefined}
              stateToneOverride={videoRecovering ? 'processing' : videoIssue ? (runtimeIssues.engine ? 'warning' : 'error') : undefined}
            />
            <ParameterSection title="普通视频" parameters={VIDEO_PARAMETERS} />
            <ParameterSection title="高级视觉" parameters={ADVANCED_PARAMETERS} startColorIndex={2} />
          </Card>
        </div>
      </div>
    </section>
  );
}

export default ParameterWorkspace;
