import {
  FolderOpenOutlined,
  PictureOutlined,
  UnorderedListOutlined,
  UploadOutlined,
} from '@ant-design/icons';
import { Alert, Button, Drawer, Empty, List, Space, Tag, Typography } from 'antd';
import { useState } from 'react';
import { DesktopPanel } from './desktop-panel';

export type PlaybackPoolSource = {
  source_path: string;
  file_name: string;
  file_size_bytes: number;
  duration_ms: number | null;
  width: number | null;
  height: number | null;
  frame_rate_fps: number | null;
  audio_sample_rate_hz: number | null;
  audio_channel_count: number | null;
  mp4_sha256: string | null;
  mp4_hash_status: 'disabled' | 'pending' | 'ready' | 'failed';
};

type PlaybackPoolPanelProps = {
  sources: readonly PlaybackPoolSource[];
  currentIndex: number | null;
  importBusy: boolean;
  importDisabled: boolean;
  error: string | null;
  onImport: () => void;
};

function formatDuration(durationMs: number | null) {
  if (durationMs === null || !Number.isFinite(durationMs) || durationMs < 0) return '时长未知';
  const totalSeconds = Math.round(durationMs / 1_000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = String(totalSeconds % 60).padStart(2, '0');
  return `${minutes}:${seconds}`;
}

function formatSourceMeta(source: PlaybackPoolSource) {
  const dimensions = source.width && source.height ? `${source.width}×${source.height}` : '分辨率未知';
  const frameRate = source.frame_rate_fps ? `${source.frame_rate_fps.toFixed(0)}fps` : '帧率未知';
  return `${formatDuration(source.duration_ms)} · ${dimensions} · ${frameRate} · ${(source.file_size_bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function PlaybackPoolPanel({
  sources,
  currentIndex,
  importBusy,
  importDisabled,
  error,
  onImport,
}: PlaybackPoolPanelProps) {
  const [drawerOpen, setDrawerOpen] = useState(false);

  const renderSource = (source: PlaybackPoolSource, index: number, showPath = false) => {
    const active = index === currentIndex;
    return (
      <List.Item
        aria-current={active ? 'true' : undefined}
        className={`desktop-source-item${active ? ' desktop-source-item-active' : ''}`}
      >
        <div className="desktop-source-item-content">
          <Space size={6} wrap>
            <Tag>第 {index + 1} 项</Tag>
            {active ? <Tag color="success">当前播放</Tag> : null}
          </Space>
          <Typography.Text strong ellipsis={{ tooltip: source.file_name }}>
            <PictureOutlined /> {source.file_name}
          </Typography.Text>
          <Typography.Text type="secondary">{formatSourceMeta(source)}</Typography.Text>
          {showPath ? <Typography.Text type="secondary" ellipsis={{ tooltip: source.source_path }}>{source.source_path}</Typography.Text> : null}
        </div>
      </List.Item>
    );
  };

  return (
    <>
      <DesktopPanel
        title="播放池"
        extra={(
          <Space size={6}>
            <Tag>共 {sources.length} 项</Tag>
            <Button
              aria-label="管理播放池"
              icon={<UnorderedListOutlined />}
              size="small"
              disabled={sources.length === 0}
              onClick={() => setDrawerOpen(true)}
            >
              管理
            </Button>
            <Button
              aria-label="导入视频"
              size="small"
              loading={importBusy}
              disabled={importDisabled}
              onClick={onImport}
            >
              <UploadOutlined /> 导入视频
            </Button>
          </Space>
        )}
        className="desktop-panel-fill desktop-source-pool"
      >
        {sources.length > 0 ? (
          <div className="desktop-source-current">
            <List
              className="desktop-source-list"
              dataSource={[...sources]}
              rowKey="source_path"
              split={false}
              renderItem={(source, index) => renderSource(source, index)}
            />
            <Button
              className="desktop-source-replace"
              icon={<UploadOutlined />}
              loading={importBusy}
              disabled={importDisabled}
              onClick={onImport}
            >
              重新选择视频
            </Button>
          </div>
        ) : (
          <Empty
            className="desktop-source-empty"
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description="尚未导入视频。可一次选择多个本地视频，系统将按播放池顺序循环播放。"
          >
            <Button
              type="primary"
              icon={<FolderOpenOutlined />}
              loading={importBusy}
              disabled={importDisabled}
              onClick={onImport}
            >
              选择视频文件
            </Button>
          </Empty>
        )}
        {error ? <Alert type="error" showIcon message={error} style={{ marginTop: 10 }} /> : null}
      </DesktopPanel>

      <Drawer
        title={`播放池管理（共 ${sources.length} 项）`}
        width={560}
        open={drawerOpen}
        onClose={() => setDrawerOpen(false)}
      >
        <List
          className="desktop-source-list desktop-source-list-drawer"
          dataSource={[...sources]}
          rowKey="source_path"
          split={false}
          renderItem={(source, index) => renderSource(source, index, true)}
        />
      </Drawer>
    </>
  );
}
