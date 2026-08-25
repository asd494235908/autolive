import {
  CheckCircleOutlined,
  ClockCircleOutlined,
  DesktopOutlined,
  FolderOpenOutlined,
  PlayCircleOutlined,
  QuestionCircleOutlined,
  ReloadOutlined,
  SoundOutlined,
  SyncOutlined,
  VideoCameraOutlined,
} from '@ant-design/icons';
import { Button, Card, Progress, Space, Switch, Tag, Tooltip, Typography } from 'antd';
import {
  ADVANCED_PARAMETERS,
  AUDIO_PARAMETERS,
  DEFAULT_CYCLE_STATES,
  DEFAULT_PLAYBACK_POOL,
  PARAMETER_PALETTE,
  PARAMETER_STATUS,
  STATUS_CARD_TEMPLATES,
  VIDEO_PARAMETERS,
} from '../data/prototype-data.js';

const { Text, Title } = Typography;

const STATUS_ICONS = {
  source: FolderOpenOutlined,
  progress: PlayCircleOutlined,
  pool: SyncOutlined,
  video: VideoCameraOutlined,
  audio: SoundOutlined,
  output: DesktopOutlined,
};

const PARAMETER_STATUS_ICONS = {
  implemented: CheckCircleOutlined,
  planned: ClockCircleOutlined,
  pendingConfirmation: QuestionCircleOutlined,
};

function clampProgress(value) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.min(100, Math.max(0, parsed)) : 0;
}

function getSourceLabel(currentSource) {
  if (typeof currentSource === 'string') return currentSource;
  return currentSource?.name || currentSource?.fileName || '尚未导入视频';
}

function getSourceSpecification(currentSource) {
  if (!currentSource?.meta) return '等待导入';
  const parts = currentSource.meta.split(' · ');
  return parts.length >= 3 ? `${parts[1]} · ${parts[2]}` : currentSource.meta;
}

function ParameterStatusTag({ status }) {
  const config = PARAMETER_STATUS[status] || PARAMETER_STATUS.pendingConfirmation;
  const StatusIcon = PARAMETER_STATUS_ICONS[status] || QuestionCircleOutlined;

  return (
    <Tag className="parameter-status-tag" color={config.tone} icon={<StatusIcon />}>
      {config.label}
    </Tag>
  );
}

function ParameterCard({ parameter, colorIndex }) {
  const accent = PARAMETER_PALETTE[colorIndex % PARAMETER_PALETTE.length];
  const isFrequencyCard = parameter.type === 'frequency';

  return (
    <Card
      className={`parameter-card${isFrequencyCard ? ' parameter-card--frequency' : ''}`}
      size="small"
      style={{ '--parameter-accent': accent }}
    >
      <div className="parameter-card__heading">
        <Text className="parameter-card__label">{parameter.label}</Text>
        <ParameterStatusTag status={parameter.status} />
      </div>
      <Text className="parameter-card__value" strong>{parameter.value}</Text>
      {isFrequencyCard ? (
        <div className="parameter-card__frequency-grid" aria-label="十二段视频空间频段权重">
          {parameter.bands.map(([frequency, weight]) => (
            <div className="parameter-card__frequency-item" key={frequency}>
              <Text>{frequency} Hz</Text>
              <Text strong>{weight.toFixed(2)}×</Text>
            </div>
          ))}
        </div>
      ) : (
        <Progress
          className="parameter-card__progress"
          percent={clampProgress(parameter.progress)}
          steps={12}
          showInfo={false}
          strokeColor={accent}
          aria-label={`${parameter.label}参数位置`}
        />
      )}
    </Card>
  );
}

function ParameterSection({ title, parameters, startColorIndex = 0 }) {
  return (
    <section className="parameter-section" aria-labelledby={`parameter-section-${title}`}>
      <div className="parameter-section__heading">
        <Title id={`parameter-section-${title}`} level={4}>{title}</Title>
        <Text type="secondary">只读快照</Text>
      </div>
      <div className="parameter-section__grid">
        {parameters.map((parameter, index) => (
          <ParameterCard
            key={parameter.key}
            parameter={parameter}
            colorIndex={startColorIndex + index}
          />
        ))}
      </div>
    </section>
  );
}

function CycleStatusCard({ title, icon, enabled, isPlaying, cycle, colorIndex }) {
  const accent = PARAMETER_PALETTE[colorIndex % PARAMETER_PALETTE.length];
  const active = enabled && isPlaying && cycle.hasNextPlan;
  const stateLabel = !enabled ? '已关闭' : !isPlaying ? '播放暂停' : cycle.stateLabel;

  return (
    <Card className="cycle-status-card" size="small" style={{ '--parameter-accent': accent }}>
      <div className="cycle-status-card__heading">
        <Space size={8}>{icon}<Text strong>{title}</Text></Space>
        <Tag color={active ? 'processing' : 'default'}>{stateLabel}</Tag>
      </div>
      <div className="cycle-status-card__meta">
        <Text type="secondary">周期范围 {cycle.range}</Text>
        <Text>已变化 {cycle.changes} 次</Text>
      </div>
      <Progress
        className="cycle-status-card__progress"
        percent={active ? clampProgress(cycle.progress) : 0}
        showInfo={false}
        strokeColor={accent}
        aria-label={`${title}下一计划进度`}
      />
    </Card>
  );
}

export function ParameterWorkspace({
  videoEnabled = true,
  audioEnabled = true,
  isPlaying = true,
  progress = 68,
  currentSource = '产品讲解-01.mp4',
  onToggleVideo,
  onResetParameters,
  onApply,
  playbackPool = DEFAULT_PLAYBACK_POOL,
  videoCycle = DEFAULT_CYCLE_STATES.video,
  audioCycle = DEFAULT_CYCLE_STATES.audio,
  outputConnected = true,
}) {
  const playbackProgress = clampProgress(progress);
  const sourceLabel = getSourceLabel(currentSource);
  const sourceSpecification = getSourceSpecification(currentSource);
  const statusValues = {
    source: { value: sourceSpecification, detail: sourceLabel },
    progress: { value: `${playbackProgress.toFixed(0)}%`, detail: isPlaying ? '正在播放' : '已暂停' },
    pool: {
      value: String(playbackPool.cycle),
      detail: `第 ${playbackPool.current}/${playbackPool.total} 项`,
    },
    video: { value: videoEnabled ? '自动低感知' : '已关闭', detail: videoEnabled ? 'FFmpeg 重新编码' : '回退源视频' },
    audio: { value: audioEnabled ? audioCycle.stateLabel : '已关闭', detail: audioEnabled ? '普通声音处理' : '保留源音轨' },
    output: { value: outputConnected ? '已连接' : '未连接', detail: '单一最终效果窗口' },
  };

  return (
    <section className="parameter-workspace" aria-labelledby="parameter-workspace-title">
      <header className="parameter-workspace__header">
        <div>
          <Title id="parameter-workspace-title" level={3}>音视频处理 · 实时参数</Title>
          <Text type="secondary">当前源的处理状态与只读参数快照</Text>
        </div>
        <Space className="parameter-workspace__actions" size={12} wrap>
          <Space size={4} wrap aria-label="参数接入状态说明">
            <Tag color="success">已接入</Tag>
            <Tag color="warning">正式需求待实现</Tag>
            <Tag>待确认</Tag>
          </Space>
          <Button onClick={() => onApply?.(audioEnabled ? 'both' : 'video')}>{audioEnabled ? '音视频同时应用' : '应用视频'}</Button>
          <Space size={8}>
            <VideoCameraOutlined />
            <Text>视频处理</Text>
            <Switch
              checked={videoEnabled}
              onChange={onToggleVideo}
              aria-label="视频处理开关"
            />
          </Space>
          <Tooltip title="恢复媒体参数默认值">
            <Button icon={<ReloadOutlined />} onClick={onResetParameters}>
              恢复默认
            </Button>
          </Tooltip>
        </Space>
      </header>

      <div className="parameter-workspace__status-grid" aria-label="当前播放与处理状态">
        {STATUS_CARD_TEMPLATES.map((template) => {
          const Icon = STATUS_ICONS[template.icon];
          const status = statusValues[template.key];
          return (
            <Card className="workspace-status-card" size="small" key={template.key}>
              <div className="workspace-status-card__heading">
                <Icon aria-hidden="true" />
                <Text type="secondary">{template.label}</Text>
              </div>
              <Tooltip title={status.value}>
                <Text className="workspace-status-card__value" strong ellipsis>{status.value}</Text>
              </Tooltip>
              <Text className="workspace-status-card__detail" type="secondary">{status.detail}</Text>
            </Card>
          );
        })}
      </div>

      <section className="parameter-workspace__cycles" aria-label="媒体随机周期状态">
        <CycleStatusCard
          title="视频周期"
          icon={<VideoCameraOutlined />}
          enabled={videoEnabled}
          isPlaying={isPlaying}
          cycle={videoCycle}
          colorIndex={0}
        />
        <CycleStatusCard
          title="声音周期"
          icon={<SoundOutlined />}
          enabled={audioEnabled}
          isPlaying={isPlaying}
          cycle={audioCycle}
          colorIndex={3}
        />
      </section>

      <div className="parameter-workspace__parameter-columns">
        <div className="parameter-workspace__parameter-column">
          <ParameterSection title="普通视频" parameters={VIDEO_PARAMETERS} />
          <ParameterSection title="高级视觉" parameters={ADVANCED_PARAMETERS} startColorIndex={2} />
        </div>
        <div className="parameter-workspace__parameter-column">
          <ParameterSection title="普通声音" parameters={AUDIO_PARAMETERS} startColorIndex={1} />
        </div>
      </div>
    </section>
  );
}

export default ParameterWorkspace;
