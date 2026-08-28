import {
  ArrowDownOutlined,
  ArrowUpOutlined,
  ClearOutlined,
  DeleteOutlined,
  EditOutlined,
  FileAddOutlined,
  PauseOutlined,
  PictureOutlined,
  PlayCircleOutlined,
  SoundOutlined,
  StopOutlined,
} from "@ant-design/icons";
import {
  Button,
  Empty,
  List,
  Popconfirm,
  Slider,
  Space,
  Tag,
  Tooltip,
} from "antd";
import { useRef, useState } from "react";

const DEFAULT_POOL = [];

const STATUS_LABELS = {
  ready: "就绪",
  playing: "播放中",
  paused: "已暂停",
  stopped: "已停止",
  probing: "读取中",
  error: "不可用",
};

function hasFilePayload(event) {
  return [...(event.dataTransfer?.types ?? [])].includes("Files");
}

export function PlaybackColumn({
  pool = DEFAULT_POOL,
  currentIndex = 0,
  playbackStatus = "ready",
  progress = 0,
  cycleCount = 1,
  currentTime = "00:00",
  duration = "00:00",
  volume = 72,
  muted = false,
  playbackActionBusy = null,
  onPoolReplaceAll,
  onPoolAppend,
  onPoolReplaceItem,
  onPoolMove,
  onPoolRemove,
  onPoolClear,
  onPlaybackAction,
  onSeek,
  onVolumeChange,
  onMuteToggle,
  onPictureInPicture,
}) {
  const [dropActive, setDropActive] = useState(false);
  const [draggedItemId, setDraggedItemId] = useState(null);
  const batchInputRef = useRef(null);
  const itemInputRef = useRef(null);
  const batchModeRef = useRef("replace");
  const replaceTargetIdRef = useRef(null);
  const currentItem = pool[currentIndex];
  const canPlay = pool.length > 0 && playbackStatus !== "probing";
  const canStart = canPlay && ["ready", "stopped"].includes(playbackStatus);
  const canPause = canPlay && playbackStatus === "playing";
  const canResume = canPlay && playbackStatus === "paused";
  const canStop = canPlay && ["playing", "paused"].includes(playbackStatus);
  const actionBusy = playbackActionBusy !== null;

  function chooseBatch(mode) {
    batchModeRef.current = mode;
    batchInputRef.current?.click();
  }

  function handleBatchSelection(event) {
    const files = [...(event.target.files ?? [])];
    event.target.value = "";
    if (!files.length) return;
    if (batchModeRef.current === "append") onPoolAppend?.(files);
    else onPoolReplaceAll?.(files);
  }

  function chooseItemReplacement(itemId) {
    replaceTargetIdRef.current = itemId;
    itemInputRef.current?.click();
  }

  function handleItemSelection(event) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (file && replaceTargetIdRef.current) onPoolReplaceItem?.(replaceTargetIdRef.current, file);
    replaceTargetIdRef.current = null;
  }

  function handlePoolDrop(event) {
    if (!hasFilePayload(event)) return;
    event.preventDefault();
    setDropActive(false);
    setDraggedItemId(null);
    const files = [...(event.dataTransfer.files ?? [])];
    if (files.length) onPoolAppend?.(files);
  }

  return (
    <aside className="playback-column" aria-label="播放池与播放控制">
      <input ref={batchInputRef} className="visually-hidden" type="file" multiple accept=".mp4,.mov,.mkv,.avi,.webm,.m4v,.ts,.m2ts,.flv,.wmv,.3gp,.mp3,.wav,.m4a,.aac,.ogg,.flac" onChange={handleBatchSelection} />
      <input ref={itemInputRef} className="visually-hidden" type="file" accept=".mp4,.mov,.mkv,.avi,.webm,.m4v,.ts,.m2ts,.flv,.wmv,.3gp,.mp3,.wav,.m4a,.aac,.ogg,.flac" onChange={handleItemSelection} />
      <section
        className={`panel-section playback-pool-section${dropActive ? " is-drop-active" : ""}`}
        aria-labelledby="playback-pool-title"
        onDragOver={(event) => {
          if (!hasFilePayload(event)) return;
          event.preventDefault();
          setDropActive(true);
        }}
        onDragLeave={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) setDropActive(false); }}
        onDrop={handlePoolDrop}
      >
        <div className="section-heading">
          <div>
            <h2 id="playback-pool-title">播放池</h2>
            <span>本地有序循环 · {pool.length}/100 项</span>
          </div>
          <Space size={6}>
            <Button size="small" icon={<FileAddOutlined />} onClick={() => chooseBatch("append")}>追加媒体</Button>
            <Popconfirm title="清空播放池？" description="清空后播放会停止。" onConfirm={onPoolClear}>
              <Button size="small" danger disabled={!pool.length} icon={<ClearOutlined />}>清空</Button>
            </Popconfirm>
          </Space>
        </div>

        {pool.length ? (
          <List
            className="playback-pool-list"
            dataSource={pool}
            renderItem={(item, index) => {
              const isCurrent = index === currentIndex;
              return (
                <List.Item
                  className={`playback-pool-item${isCurrent ? " is-current" : ""}${draggedItemId === item.id ? " is-dragging" : ""}`}
                  draggable
                  onDragStart={(event) => { setDraggedItemId(item.id); event.dataTransfer.effectAllowed = "move"; }}
                  onDragOver={(event) => { if (draggedItemId) event.preventDefault(); }}
                  onDrop={(event) => {
                    if (!draggedItemId) return;
                    event.preventDefault();
                    event.stopPropagation();
                    onPoolMove?.(draggedItemId, index);
                    setDraggedItemId(null);
                    setDropActive(false);
                  }}
                  onDragEnd={() => { setDraggedItemId(null); setDropActive(false); }}
                >
                  <div
                    className="pool-item-button"
                    aria-current={isCurrent ? "true" : undefined}
                  >
                    <span className="pool-item-order">{String(index + 1).padStart(2, "0")}</span>
                    <span className="pool-item-copy">
                      <strong title={item.name}>{item.name}</strong>
                      <small>{item.meta || "等待读取媒体信息"}</small>
                    </span>
                    <span className="pool-item-actions">
                      <Tooltip title="上移"><Button aria-label={`上移 ${item.name}`} size="small" type="text" icon={<ArrowUpOutlined />} disabled={index === 0} onClick={() => onPoolMove?.(item.id, index - 1)} /></Tooltip>
                      <Tooltip title="下移"><Button aria-label={`下移 ${item.name}`} size="small" type="text" icon={<ArrowDownOutlined />} disabled={index === pool.length - 1} onClick={() => onPoolMove?.(item.id, index + 1)} /></Tooltip>
                      <Tooltip title="替换"><Button aria-label={`替换 ${item.name}`} size="small" type="text" icon={<EditOutlined />} onClick={() => chooseItemReplacement(item.id)} /></Tooltip>
                      <Popconfirm title="删除此媒体？" onConfirm={() => onPoolRemove?.(item.id)}>
                        <Tooltip title="删除"><Button aria-label={`删除 ${item.name}`} size="small" type="text" danger icon={<DeleteOutlined />} /></Tooltip>
                      </Popconfirm>
                    </span>
                  </div>
                </List.Item>
              );
            }}
          />
        ) : (
          <Empty
            className="playback-pool-empty"
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description="尚未导入本地媒体"
          >
            <Button type="primary" icon={<FileAddOutlined />} onClick={() => chooseBatch("replace")}>
              选择 1–100 个媒体
            </Button>
          </Empty>
        )}

        <div className="pool-drop-hint">拖放媒体到此处追加 · 拖动条目排序</div>
        {pool.length ? <Button className="replace-pool-button" block icon={<FileAddOutlined />} onClick={() => chooseBatch("replace")}>整批替换</Button> : null}

        {currentItem && (
          <div className="current-media-summary">
            <span>当前项</span>
            <strong title={currentItem.name}>{currentItem.name}</strong>
            <Tag color="processing">{STATUS_LABELS[playbackStatus] || "就绪"}</Tag>
          </div>
        )}
      </section>

      <section className="panel-section playback-controls-section" aria-labelledby="playback-controls-title">
        <div className="section-heading compact-heading">
          <h2 id="playback-controls-title">播放控制</h2>
          <span>{currentTime} / {duration}</span>
        </div>
        <dl className="playback-facts">
          <div><dt>播放进度</dt><dd>{Math.round(progress)}%</dd></div>
          <div><dt>循环次数</dt><dd>{cycleCount} · 第 {pool.length ? currentIndex + 1 : 0}/{pool.length} 项</dd></div>
        </dl>
        <Slider
          aria-label="播放进度"
          min={0}
          max={100}
          value={progress}
          tooltip={{ formatter: (value) => `${value}%` }}
          disabled={!canPlay}
          onChange={onSeek}
        />

        <div className="playback-action-row">
          <Button
            type="primary"
            icon={<PlayCircleOutlined />}
            loading={playbackActionBusy === "play"}
            disabled={!canStart || (actionBusy && playbackActionBusy !== "play")}
            onClick={() => onPlaybackAction?.("play")}
          >
            播放
          </Button>
          <Button
            icon={<PauseOutlined />}
            loading={playbackActionBusy === "pause"}
            disabled={!canPause || (actionBusy && playbackActionBusy !== "pause")}
            onClick={() => onPlaybackAction?.("pause")}
          >
            暂停
          </Button>
          <Button
            icon={<PlayCircleOutlined />}
            loading={playbackActionBusy === "resume"}
            disabled={!canResume || (actionBusy && playbackActionBusy !== "resume")}
            onClick={() => onPlaybackAction?.("resume")}
          >
            继续
          </Button>
          <Button
            icon={<StopOutlined />}
            loading={playbackActionBusy === "stop"}
            disabled={!canStop || (actionBusy && playbackActionBusy !== "stop")}
            onClick={() => onPlaybackAction?.("stop")}
          >
            停止
          </Button>
          <Tooltip title="在画中画窗口查看当前画面">
            <Button
              aria-label="打开画中画"
              icon={<PictureOutlined />}
              disabled={!canPlay || actionBusy}
              onClick={onPictureInPicture}
            />
          </Tooltip>
        </div>

        <div className="volume-control-row">
          <Tooltip title={muted ? "取消静音" : "静音"}>
            <Button
              aria-label={muted ? "取消静音" : "静音"}
              type="text"
              icon={<SoundOutlined />}
              onClick={onMuteToggle}
            />
          </Tooltip>
          <Slider
            aria-label="播放音量"
            min={0}
            max={100}
            value={muted ? 0 : volume}
            onChange={onVolumeChange}
          />
          <span>{muted ? 0 : volume}%</span>
        </div>
      </section>

    </aside>
  );
}
