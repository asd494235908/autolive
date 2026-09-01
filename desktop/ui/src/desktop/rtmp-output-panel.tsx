import { Button, Input, InputNumber, Select, Space, Switch, Tag, Typography } from 'antd';

export type RtmpOutputPanelConfig = {
  target_url: string;
  video_enabled: boolean;
  audio_enabled: boolean;
  width: number;
  height: number;
  fps: number;
  video_bitrate_kbps: number;
  audio_bitrate_kbps: number;
};

export type RtmpOutputPanelStatus = {
  state: string;
  session_generation?: number;
  target_url?: string | null;
  video_enabled?: boolean;
  audio_enabled?: boolean;
  width?: number | null;
  height?: number | null;
  fps?: number | null;
  video_bitrate_kbps?: number | null;
  audio_bitrate_kbps?: number | null;
  current_bitrate_kbps?: number | null;
  source_identity?: {
    playback_generation: number;
    source_media_index: number;
    loop_index: number;
    source_duration_ms?: number | null;
    source_position_ms: number;
  } | null;
  video_filter_backend?: string | null;
  error?: string | null;
  encoder?: string | null;
  process_id?: number | null;
  retry_count?: number;
  published_ms?: number;
  output_bytes?: number;
  last_progress_ms?: number | null;
  dropped_audio_chunks?: number;
  error_code?: string | null;
};

type Props = {
  config: RtmpOutputPanelConfig;
  status: RtmpOutputPanelStatus | null;
  busy: boolean;
  error: string | null;
  onChange: (patch: Partial<RtmpOutputPanelConfig>) => void;
  onValidate: () => void;
  onStart: () => void;
  onStop: () => void;
};

function stateLabel(state: string | undefined): { label: string; color: string } {
  switch (state) {
    case 'publishing':
      return { label: '推流中', color: 'success' };
    case 'starting':
    case 'validating':
    case 'reconnecting':
      return { label: '处理中', color: 'processing' };
    case 'failed':
      return { label: '失败', color: 'error' };
    case 'stopping':
      return { label: '停止中', color: 'warning' };
    default:
      return { label: '未推流', color: 'default' };
  }
}

function formatPublishedDuration(milliseconds: number): string {
  const totalSeconds = Math.max(0, Math.floor(milliseconds / 1_000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

function filterBackendLabel(backend: string): string {
  switch (backend) {
    case 'gpu83':
      return 'GPU83';
    case 'cpu4':
      return 'CPU4';
    case 'original':
      return '原始链';
    default:
      return backend;
  }
}

export function RtmpOutputPanel({
  config,
  status,
  busy,
  error,
  onChange,
  onValidate,
  onStart,
  onStop,
}: Props) {
  const state = stateLabel(status?.state);
  const trackCount = Number(config.video_enabled) + Number(config.audio_enabled);

  return (
    <div className="desktop-rtmp-output-panel" aria-label="ZLMediaKit RTMP 推流">
      <div className="desktop-status-line">
        <span>ZLMediaKit RTMP</span>
        <Tag color={state.color}>{state.label}</Tag>
        <Tag color="warning">正式需求·待实施/未接入</Tag>
      </div>
      <Typography.Text className="desktop-muted">
        画面直接读取当前媒体源，声音分流最终 PCM 推送到用户指定的 ZLMediaKit 流媒体服务器，不捕获桌面或窗口。
      </Typography.Text>
      <Typography.Text className="desktop-muted">
        含声音时分流最终 PCM；固定话术当前由 WebView 系统声音播放，不进入推流音轨。
      </Typography.Text>
      <Typography.Text className="desktop-muted">
        含声音推流需要先启用 PortAudio 最终声音出口。
      </Typography.Text>
      <Space direction="vertical" size="small" style={{ width: '100%', marginTop: 10 }}>
        <Input
          aria-label="RTMP 或 RTMPS 推流地址"
          placeholder="rtmp://127.0.0.1/live/stream"
          value={config.target_url}
          maxLength={2048}
          disabled={busy}
          onChange={(event) => onChange({ target_url: event.target.value })}
        />
        <Space wrap>
          <label>
            <Switch
              size="small"
              checked={config.video_enabled}
              disabled={busy || trackCount <= 1}
              onChange={(checked) => onChange({ video_enabled: checked })}
            />{' '}
            画面
          </label>
          <label>
            <Switch
              size="small"
              checked={config.audio_enabled}
              disabled={busy || trackCount <= 1}
              onChange={(checked) => onChange({ audio_enabled: checked })}
            />{' '}
            声音
          </label>
        </Space>
        {config.video_enabled ? (
          <Space wrap>
            <Typography.Text>宽</Typography.Text>
            <InputNumber aria-label="输出宽度" min={160} max={3840} step={2} value={config.width} disabled={busy} onChange={(value) => typeof value === 'number' && onChange({ width: value })} />
            <Typography.Text>高</Typography.Text>
            <InputNumber aria-label="输出高度" min={90} max={2160} step={2} value={config.height} disabled={busy} onChange={(value) => typeof value === 'number' && onChange({ height: value })} />
            <Typography.Text>FPS</Typography.Text>
            <Select aria-label="输出帧率" value={config.fps} disabled={busy} options={[25, 30, 50, 60].map((fps) => ({ value: fps, label: `${fps} FPS`, disabled: fps >= 50 }))} onChange={(value) => onChange({ fps: value })} />
            <Typography.Text>视频 kbps</Typography.Text>
            <InputNumber aria-label="视频码率" min={128} max={50000} value={config.video_bitrate_kbps} disabled={busy} onChange={(value) => typeof value === 'number' && onChange({ video_bitrate_kbps: value })} />
          </Space>
        ) : null}
        {config.audio_enabled ? (
          <Space>
            <Typography.Text>音频 kbps</Typography.Text>
            <InputNumber aria-label="音频码率" min={32} max={512} value={config.audio_bitrate_kbps} disabled={busy} onChange={(value) => typeof value === 'number' && onChange({ audio_bitrate_kbps: value })} />
          </Space>
        ) : null}
        {status?.target_url?.endsWith('/<redacted>') ? <Typography.Text className="desktop-muted">目标：{status.target_url}</Typography.Text> : null}
        {status && (status.video_enabled || status.audio_enabled) ? <Typography.Text className="desktop-muted">轨道：{status.video_enabled ? '画面' : ''}{status.video_enabled && status.audio_enabled ? ' + ' : ''}{status.audio_enabled ? '声音' : ''}{status.width && status.height && status.fps ? ` · ${status.width}×${status.height}@${status.fps}` : ''}</Typography.Text> : null}
        {status?.encoder ? <Typography.Text className="desktop-muted">编码器：{status.encoder}</Typography.Text> : null}
        {status?.video_filter_backend ? <Typography.Text className="desktop-muted">视觉链：{filterBackendLabel(status.video_filter_backend)}</Typography.Text> : null}
        {status?.source_identity ? <Typography.Text className="desktop-muted">源：第 {status.source_identity.source_media_index + 1} 项 · 循环 {status.source_identity.loop_index} · {Math.round(status.source_identity.source_position_ms / 1000)}s</Typography.Text> : null}
        {status && (status.video_bitrate_kbps || status.audio_bitrate_kbps) ? <Typography.Text className="desktop-muted">目标码率：{status.video_bitrate_kbps ? `${status.video_bitrate_kbps}k 视频` : ''}{status.video_bitrate_kbps && status.audio_bitrate_kbps ? ' + ' : ''}{status.audio_bitrate_kbps ? `${status.audio_bitrate_kbps}k 音频` : ''}</Typography.Text> : null}
        {typeof status?.current_bitrate_kbps === 'number' ? <Typography.Text className="desktop-muted">当前码率：{status.current_bitrate_kbps} kbps</Typography.Text> : null}
        {typeof status?.published_ms === 'number' && status.published_ms > 0 ? <Typography.Text className="desktop-muted">已推送：{formatPublishedDuration(status.published_ms)}</Typography.Text> : null}
        {typeof status?.retry_count === 'number' && status.retry_count > 0 ? <Typography.Text className="desktop-muted">重试：第 {status.retry_count} 次</Typography.Text> : null}
        {typeof status?.output_bytes === 'number' && status.output_bytes > 0 ? <Typography.Text className="desktop-muted">已输出：{Math.round(status.output_bytes / 1024)} KiB</Typography.Text> : null}
        {typeof status?.dropped_audio_chunks === 'number' && status.dropped_audio_chunks > 0 ? <Typography.Text type="warning">音频队列丢弃：{status.dropped_audio_chunks} 个分片，推流将受控重建</Typography.Text> : null}
        {status?.error_code ? <Typography.Text className="desktop-muted">错误码：{status.error_code}</Typography.Text> : null}
        {error || status?.error ? <Typography.Text type="danger">{error ?? status?.error}</Typography.Text> : null}
        <Space>
          <Button size="small" onClick={onValidate} disabled={busy}>校验地址</Button>
          {status?.state === 'publishing' || status?.state === 'starting' || status?.state === 'reconnecting' ? (
            <Button size="small" danger onClick={onStop} loading={busy} disabled={busy}>停止推流</Button>
          ) : (
            <Button size="small" type="primary" onClick={onStart} loading={busy} disabled={busy || trackCount === 0}>开始推流</Button>
          )}
        </Space>
      </Space>
    </div>
  );
}
