import {
  ArrowDownOutlined,
  ArrowUpOutlined,
  AudioOutlined,
  ClearOutlined,
  DeleteOutlined,
  EditOutlined,
  FolderOpenOutlined,
  PictureOutlined,
  PlusOutlined,
  UploadOutlined,
} from '@ant-design/icons';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { Alert, Button, Empty, List, Popconfirm, Space, Tag, Tooltip, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import type { PointerEvent as ReactPointerEvent } from 'react';
import { DesktopPanel } from './desktop-panel';

export type PlaybackPoolSource = {
  source_path: string;
  media_kind: 'video' | 'audio';
  playback_reference: string;
  compatibility_mode: 'direct' | 'remuxed' | 'transcoded';
  file_name: string;
  file_size_bytes: number;
  duration_ms: number | null;
  width: number | null;
  height: number | null;
  frame_rate_fps: number | null;
  audio_sample_rate_hz: number | null;
  audio_channel_count: number | null;
  audio_start_ms: number | null;
  audio_end_ms: number | null;
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
  onAppend: () => void;
  onDropFiles: (paths: string[]) => void;
  onReplace: (sourcePath: string) => void;
  onReorder: (sourcePaths: string[]) => void;
  onRemove: (sourcePath: string) => void;
  onClear: () => void;
};

type SortGesture = {
  pointerId: number;
  fromIndex: number;
  toIndex: number;
};

function formatDuration(durationMs: number | null) {
  if (durationMs === null || !Number.isFinite(durationMs) || durationMs < 0) return '时长未知';
  const totalSeconds = Math.round(durationMs / 1_000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = String(totalSeconds % 60).padStart(2, '0');
  return `${minutes}:${seconds}`;
}

function formatSourceMeta(source: PlaybackPoolSource) {
  if (source.media_kind === 'audio') {
    const sampleRate = source.audio_sample_rate_hz
      ? `${(source.audio_sample_rate_hz / 1_000).toFixed(source.audio_sample_rate_hz % 1_000 === 0 ? 0 : 1)}kHz`
      : '采样率未知';
    const channels = source.audio_channel_count ? `${source.audio_channel_count} 声道` : '声道未知';
    return `${formatDuration(source.duration_ms)} · ${sampleRate} · ${channels} · ${(source.file_size_bytes / 1024 / 1024).toFixed(1)} MB`;
  }
  const dimensions = source.width && source.height ? `${source.width}×${source.height}` : '分辨率未知';
  const frameRate = source.frame_rate_fps ? `${source.frame_rate_fps.toFixed(0)}fps` : '帧率未知';
  return `${formatDuration(source.duration_ms)} · ${dimensions} · ${frameRate} · ${(source.file_size_bytes / 1024 / 1024).toFixed(1)} MB`;
}

function moveSourcePath(sources: readonly PlaybackPoolSource[], fromIndex: number, toIndex: number) {
  const paths = sources.map((source) => source.source_path);
  const [moved] = paths.splice(fromIndex, 1);
  if (moved === undefined) return paths;
  paths.splice(toIndex, 0, moved);
  return paths;
}

export function PlaybackPoolPanel({
  sources,
  currentIndex,
  importBusy,
  importDisabled,
  error,
  onImport,
  onAppend,
  onDropFiles,
  onReplace,
  onReorder,
  onRemove,
  onClear,
}: PlaybackPoolPanelProps) {
  const [dropActive, setDropActive] = useState(false);
  const [sortGesture, setSortGesture] = useState<SortGesture | null>(null);
  const dropFilesRef = useRef(onDropFiles);
  const dropDisabledRef = useRef(importDisabled);
  const sortGestureRef = useRef<SortGesture | null>(null);

  useEffect(() => {
    dropFilesRef.current = onDropFiles;
    dropDisabledRef.current = importDisabled;
  }, [importDisabled, onDropFiles]);

  useEffect(() => {
    if (!('__TAURI_INTERNALS__' in window)) return undefined;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === 'enter' || payload.type === 'over') {
        setDropActive(!dropDisabledRef.current);
        return;
      }
      setDropActive(false);
      if (payload.type === 'drop' && payload.paths.length > 0 && !dropDisabledRef.current) {
        dropFilesRef.current(payload.paths);
      }
    }).then((nextUnlisten) => {
      if (disposed) nextUnlisten();
      else unlisten = nextUnlisten;
    }, () => {
      if (!disposed) setDropActive(false);
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  function startPointerSort(event: ReactPointerEvent<HTMLElement>, index: number) {
    if (importDisabled) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    const next = { pointerId: event.pointerId, fromIndex: index, toIndex: index };
    sortGestureRef.current = next;
    setSortGesture(next);
  }

  function updatePointerSort(event: ReactPointerEvent<HTMLElement>) {
    const current = sortGestureRef.current;
    if (!current || current.pointerId !== event.pointerId) return;
    const target = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('[data-pool-index]');
    const toIndex = Number(target?.dataset.poolIndex);
    if (!Number.isInteger(toIndex) || toIndex < 0 || toIndex >= sources.length || toIndex === current.toIndex) return;
    const next = { ...current, toIndex };
    sortGestureRef.current = next;
    setSortGesture(next);
  }

  function finishPointerSort(event: ReactPointerEvent<HTMLElement>, cancelled = false) {
    const current = sortGestureRef.current;
    if (!current || current.pointerId !== event.pointerId) return;
    sortGestureRef.current = null;
    setSortGesture(null);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    if (!cancelled && current.fromIndex !== current.toIndex) {
      onReorder(moveSourcePath(sources, current.fromIndex, current.toIndex));
    }
  }

  function cancelPointerSort() {
    sortGestureRef.current = null;
    setSortGesture(null);
  }

  function moveByKeyboard(index: number, offset: -1 | 1) {
    const toIndex = index + offset;
    if (toIndex < 0 || toIndex >= sources.length) return;
    onReorder(moveSourcePath(sources, index, toIndex));
  }

  const renderSource = (source: PlaybackPoolSource, index: number) => {
    const active = index === currentIndex;
    const MediaIcon = source.media_kind === 'audio' ? AudioOutlined : PictureOutlined;
    const sorting = sortGesture?.fromIndex === index;
    const sortTarget = sortGesture?.toIndex === index;
    return (
      <List.Item
        aria-current={active ? 'true' : undefined}
        className={`desktop-source-item${active ? ' desktop-source-item-active' : ''}${sorting ? ' desktop-source-item-sorting' : ''}${sortTarget ? ' desktop-source-item-sort-target' : ''}`}
        data-pool-index={index}
      >
        <div
          aria-label={`拖动第 ${index + 1} 项排序`}
          className="desktop-source-item-content"
          onPointerDown={(event) => startPointerSort(event, index)}
          onPointerMove={updatePointerSort}
          onPointerUp={finishPointerSort}
          onPointerCancel={(event) => finishPointerSort(event, true)}
          onLostPointerCapture={cancelPointerSort}
        >
          <Space size={6} wrap>
            <Tag>第 {index + 1} 项</Tag>
            <Tag>{source.media_kind === 'audio' ? '音频' : '视频'}</Tag>
            {active ? <Tag color="success">当前项</Tag> : null}
          </Space>
          <Typography.Text strong ellipsis={{ tooltip: source.file_name }}>
            <MediaIcon /> {source.file_name}
          </Typography.Text>
          <Typography.Text type="secondary">{formatSourceMeta(source)}</Typography.Text>
        </div>
        <Space className="desktop-source-actions" size={2}>
          <Tooltip title="上移"><Button aria-label={`上移第 ${index + 1} 项`} size="small" disabled={importDisabled || index === 0} icon={<ArrowUpOutlined />} type="text" onClick={() => moveByKeyboard(index, -1)} /></Tooltip>
          <Tooltip title="下移"><Button aria-label={`下移第 ${index + 1} 项`} size="small" disabled={importDisabled || index === sources.length - 1} icon={<ArrowDownOutlined />} type="text" onClick={() => moveByKeyboard(index, 1)} /></Tooltip>
          <Tooltip title="替换媒体"><Button aria-label={`替换第 ${index + 1} 项媒体`} size="small" disabled={importDisabled} icon={<EditOutlined />} type="text" onClick={() => onReplace(source.source_path)} /></Tooltip>
          <Popconfirm title="删除这个媒体？" description={sources.length === 1 ? '播放池将变为空并停止。' : '播放将停止，剩余条目会从第一项重新就绪。'} onConfirm={() => onRemove(source.source_path)}>
            <Tooltip title="删除媒体"><Button aria-label={`删除第 ${index + 1} 项媒体`} size="small" danger disabled={importDisabled} icon={<DeleteOutlined />} type="text" /></Tooltip>
          </Popconfirm>
        </Space>
      </List.Item>
    );
  };

  return (
    <DesktopPanel
      title={<div className="desktop-source-title"><strong>播放池</strong><span>本地有序循环 · {sources.length}/100 项</span></div>}
      extra={(
        <Space className="desktop-source-header-actions" size={6}>
          <Button aria-label="追加媒体" size="small" icon={<PlusOutlined />} loading={importBusy} disabled={importDisabled || sources.length >= 100} onClick={onAppend}>追加媒体</Button>
          <Popconfirm title="清空播放池？" description="播放将停止，所有条目都会移除。" onConfirm={onClear}>
            <Button aria-label="清空播放池" size="small" danger icon={<ClearOutlined />} disabled={importDisabled || sources.length === 0}>清空</Button>
          </Popconfirm>
        </Space>
      )}
      className={`desktop-panel-fill desktop-source-pool${dropActive ? ' desktop-source-pool-drop-active' : ''}`}
    >
        <div className="desktop-source-drop-zone">
          {dropActive ? <div className="desktop-source-drop-overlay" role="status" aria-live="polite"><UploadOutlined /> 松开鼠标，追加到播放池</div> : null}
          {sources.length > 0 ? (
            <div className="desktop-source-current">
              <List className="desktop-source-list" dataSource={[...sources]} rowKey="source_path" split={false} renderItem={renderSource} />
              <Typography.Text className="desktop-source-drop-hint" type="secondary">拖放媒体到此处追加 · 追加后最多可保留 100 项 · 拖动条目排序</Typography.Text>
              <Button className="desktop-source-replace" icon={<UploadOutlined />} loading={importBusy} disabled={importDisabled} onClick={onImport}>整批替换</Button>
            </div>
          ) : (
            <Empty className="desktop-source-empty" image={Empty.PRESENTED_IMAGE_SIMPLE} description="尚未导入本地媒体">
              <Button type="primary" icon={<FolderOpenOutlined />} loading={importBusy} disabled={importDisabled} onClick={onImport}>导入媒体</Button>
            </Empty>
          )}
        </div>
        {error ? <Alert type="error" showIcon message={error} style={{ marginTop: 10 }} /> : null}
    </DesktopPanel>
  );
}
