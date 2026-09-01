import { Button, Descriptions, Space, Tag, Typography } from 'antd';

export type VirtualCameraOutputStatus = {
  state: string;
  config: {
    device_name: string;
    pixel_format: string;
    width: number;
    height: number;
    fps: number;
    zero_copy: boolean;
  };
  generation: number;
  gpu?: {
    capture_api: string;
    adapter_luid?: string;
    adapter_name: string;
    vendor_id: number;
    device_id: number;
    feature_level: string;
    is_warp: boolean;
    gpu_scale: boolean;
    gpu_color_convert: boolean;
    transport: string;
    zero_copy: boolean;
  } | null;
  downstream_client_count?: number | null;
  metrics: {
    frames_submitted: number;
    frames_delivered: number;
    frames_dropped: number;
    stale_frames_rejected: number;
    readback_count: number;
    readback_p99_us?: number | null;
  };
  last_error?: string | null;
};

type Props = {
  status: VirtualCameraOutputStatus | null;
  busy: boolean;
  error: string | null;
  onInstall: () => void;
  onStart: () => void;
  onStop: () => void;
};

function stateLabel(state: string | undefined): { label: string; color: string } {
  switch (state) {
    case 'Streaming':
      return { label: '输出中', color: 'success' };
    case 'Ready':
      return { label: '已就绪', color: 'processing' };
    case 'Starting':
    case 'Recovering':
    case 'Stopping':
      return { label: '处理中', color: 'processing' };
    case 'Installed':
      return { label: '已安装', color: 'warning' };
    case 'Failed':
      return { label: '失败', color: 'error' };
    default:
      return { label: '未接入', color: 'default' };
  }
}

function pixelFormatLabel(value: string | undefined): string {
  return value?.toLowerCase() === 'yuy2' ? 'YUY2' : value ?? 'YUY2';
}

export function VirtualCameraOutputPanel({ status, busy, error, onInstall, onStart, onStop }: Props) {
  const state = stateLabel(status?.state);
  const active = ['Streaming', 'Starting', 'Ready', 'Recovering'].includes(status?.state ?? '');
  const config = status?.config;
  const gpu = status?.gpu;

  return (
    <div className="desktop-virtual-camera-output-panel" aria-label="AkVirtualCamera 虚拟摄像头">
      <div className="desktop-status-line">
        <span>虚拟摄像头</span>
        <Tag color={state.color}>{state.label}</Tag>
      </div>
      <Typography.Text className="desktop-muted">
        输出最终效果窗口；请在 Chrome、Teams、Zoom 或其他下游应用中选择 GpAutoLive Camera。
      </Typography.Text>
      <Descriptions size="small" column={1} style={{ marginTop: 8 }}>
        <Descriptions.Item label="设备">{config?.device_name ?? 'GpAutoLive Camera'}</Descriptions.Item>
        <Descriptions.Item label="规格">{config ? `${config.width}×${config.height}@${config.fps} · ${pixelFormatLabel(config.pixel_format)}` : 'YUY2 1280×720@30fps'}</Descriptions.Item>
        <Descriptions.Item label="捕获 API">{gpu?.capture_api ?? '等待 WGC/D3D11 准入'}</Descriptions.Item>
        <Descriptions.Item label="GPU">{gpu ? `${gpu.adapter_name} · Vendor 0x${gpu.vendor_id.toString(16).padStart(4, '0')} · Device 0x${gpu.device_id.toString(16).padStart(4, '0')} · ${gpu.feature_level}${gpu.adapter_luid ? ` · LUID ${gpu.adapter_luid}` : ''}` : '等待 WGC/D3D11 准入'}</Descriptions.Item>
        <Descriptions.Item label="GPU 转换">{gpu ? `${gpu.gpu_scale ? 'GPU 缩放' : '非 GPU 缩放'} · ${gpu.gpu_color_convert ? 'GPU 色彩转换' : '非 GPU 色彩转换'}` : '等待 GPU 事实'}</Descriptions.Item>
        <Descriptions.Item label="传输">{gpu?.transport ?? 'AkVirtualCamera CPU raw（zero_copy=false）'}</Descriptions.Item>
        {status ? <Descriptions.Item label="下游客户端">{status.downstream_client_count == null ? '未探测' : status.downstream_client_count}</Descriptions.Item> : null}
        {status ? <Descriptions.Item label="帧推进">提交 {status.metrics.frames_submitted} · 投递 {status.metrics.frames_delivered} · 丢弃 {status.metrics.frames_dropped}</Descriptions.Item> : null}
        {status?.metrics.readback_p99_us != null ? <Descriptions.Item label="回读 P99">{status.metrics.readback_p99_us} μs</Descriptions.Item> : null}
      </Descriptions>
      {error || status?.last_error ? <Typography.Text type="danger">{error ?? status?.last_error}</Typography.Text> : null}
      <Space style={{ marginTop: 8 }} wrap>
        <Button size="small" onClick={onInstall} loading={busy}>安装/修复</Button>
        {active ? (
          <Button size="small" danger onClick={onStop} loading={busy}>停止输出</Button>
        ) : (
          <Button size="small" type="primary" onClick={onStart} loading={busy} disabled={!status || status.state === 'Unavailable'}>开始输出</Button>
        )}
      </Space>
    </div>
  );
}
