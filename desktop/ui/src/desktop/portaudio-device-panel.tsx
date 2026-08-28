import { AudioOutlined } from '@ant-design/icons';
import { Button, Select, Space, Tag, Typography } from 'antd';

import { CompactNumberField } from './compact-number-field';
import { DesktopPanel } from './desktop-panel';

type OutputDevice = {
  id: string;
  name: string;
  host_api: string;
};

const SYSTEM_DEFAULT_OUTPUT_DEVICE_ID = '__system_default__';

export function PortAudioDevicePanel({
  available,
  running,
  actualOutput,
  processingStatus,
  hostApi,
  hasAsioDevice,
  devices,
  deviceId,
  memoryBufferKib,
  appliedDeviceId,
  appliedMemoryBufferKib,
  busy,
  devicesRefreshing,
  onHostApiChange,
  onDeviceChange,
  onMemoryBufferChange,
  onMemoryBufferBlur,
  onRefreshDevices,
  onApply,
  onTestTone,
}: {
  available: boolean;
  running: boolean;
  actualOutput: string;
  processingStatus: string;
  hostApi: string;
  hasAsioDevice: boolean;
  devices: readonly OutputDevice[];
  deviceId: string | null;
  memoryBufferKib: number | null;
  appliedDeviceId: string | null;
  appliedMemoryBufferKib: number;
  busy: boolean;
  devicesRefreshing: boolean;
  onHostApiChange: (value: string) => void;
  onDeviceChange: (value: string | null) => void;
  onMemoryBufferChange: (value: number | null) => void;
  onMemoryBufferBlur: () => void;
  onRefreshDevices: () => void;
  onApply: () => void;
  onTestTone: () => void;
}) {
  const visibleDevices = devices.filter((device) => hostApi === 'all' || device.host_api.trim().toLowerCase() === hostApi);
  const invalidBuffer = typeof memoryBufferKib !== 'number' || !Number.isInteger(memoryBufferKib) || memoryBufferKib < 128 || memoryBufferKib > 2048;
  const validDevice = deviceId === null
    ? hostApi === 'all'
    : visibleDevices.some((device) => device.id === deviceId);
  const configurationDirty = deviceId !== appliedDeviceId
    || memoryBufferKib !== appliedMemoryBufferKib
    || (hostApi !== 'all' && deviceId === null);
  const appliedDevice = appliedDeviceId === null
    ? null
    : devices.find((device) => device.id === appliedDeviceId);
  const appliedConfiguration = appliedDevice
    ? `${appliedDevice.name} · ${appliedDevice.host_api} · ${appliedMemoryBufferKib} KiB`
    : appliedDeviceId === null
      ? `系统默认输出设备 · ${appliedMemoryBufferKib} KiB`
      : `设备 #${appliedDeviceId} · ${appliedMemoryBufferKib} KiB`;
  const applyDisabled = !available || invalidBuffer || !validDevice || (!configurationDirty && running) || busy || devicesRefreshing;
  const deviceOptions = [
    ...(hostApi === 'all' ? [{ value: SYSTEM_DEFAULT_OUTPUT_DEVICE_ID, label: '系统默认输出设备' }] : []),
    ...visibleDevices.map((device) => ({ value: device.id, label: `${device.name} · ${device.host_api}` })),
  ];
  return (
    <DesktopPanel
      title="PortAudio 设备"
      extra={<Tag color={running ? 'success' : available ? 'warning' : 'default'} icon={<AudioOutlined />}>{running ? '运行中' : available ? '已旁路' : '不可用'}</Tag>}
    >
      <div className="desktop-portaudio-form">
        <div className="feature-drawer-field">
          <label htmlFor="home-portaudio-host-api">Host API</label>
          <Select id="home-portaudio-host-api" aria-label="PortAudio Host API" size="small" value={hostApi} options={[
            { value: 'all', label: '全部 Host API' },
            { value: 'wasapi', label: 'WASAPI' },
            { value: 'mme', label: 'MME' },
            { value: 'dsound', label: 'DirectSound' },
            { value: 'wdmks', label: 'WDMKS' },
            { value: 'asio', label: hasAsioDevice ? 'ASIO' : 'ASIO（未检测到设备）', disabled: !hasAsioDevice },
          ]} onChange={onHostApiChange} />
        </div>
        <div className="feature-drawer-field">
          <label htmlFor="home-portaudio-output-device">输出设备</label>
          <Select id="home-portaudio-output-device" aria-label="PortAudio 输出设备" allowClear size="small" loading={devicesRefreshing} placeholder="请选择输出设备" notFoundContent={devicesRefreshing ? '正在获取最新设备…' : undefined} value={deviceId ?? SYSTEM_DEFAULT_OUTPUT_DEVICE_ID} options={deviceOptions} onOpenChange={(open) => { if (open) onRefreshDevices(); }} onChange={(value) => onDeviceChange(value === undefined || value === SYSTEM_DEFAULT_OUTPUT_DEVICE_ID ? null : String(value))} />
        </div>
        <div className="feature-drawer-field">
          <label>内存缓冲</label>
          <CompactNumberField ariaLabel="PortAudio 内存缓冲区大小" unit="KiB" min={128} max={2048} value={memoryBufferKib} onChange={onMemoryBufferChange} onBlur={onMemoryBufferBlur} />
          <Typography.Text className="desktop-portaudio-helper">128–2048 KiB</Typography.Text>
        </div>
      </div>
      <div className="desktop-status-line desktop-portaudio-status"><Typography.Text className="desktop-muted">配置目标</Typography.Text><strong title={appliedConfiguration}>{appliedConfiguration}</strong></div>
      <div className="desktop-status-line desktop-portaudio-status"><Typography.Text className="desktop-muted">实际出口</Typography.Text><strong title={actualOutput}>{actualOutput}</strong></div>
      <div className="desktop-status-line desktop-portaudio-status"><Typography.Text className="desktop-muted">处理状态</Typography.Text><strong title={configurationDirty ? '待应用' : processingStatus}>{configurationDirty ? '待应用' : processingStatus}</strong></div>
      <Space.Compact className="desktop-portaudio-actions" block>
        <Button disabled={applyDisabled} loading={busy} onClick={onApply}>应用设置</Button>
        <Button disabled={!running || busy || devicesRefreshing} loading={busy} onClick={onTestTone}>播放测试音</Button>
      </Space.Compact>
    </DesktopPanel>
  );
}
