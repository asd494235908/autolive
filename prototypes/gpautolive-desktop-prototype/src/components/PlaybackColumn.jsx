import {
  FileAddOutlined,
  PauseOutlined,
  PictureOutlined,
  PlayCircleOutlined,
  ReloadOutlined,
  SoundOutlined,
  StopOutlined,
  UnorderedListOutlined,
} from "@ant-design/icons";
import {
  Button,
  Drawer,
  Empty,
  InputNumber,
  List,
  Slider,
  Space,
  Switch,
  Tag,
  Tooltip,
} from "antd";
import { useState } from "react";

const DEFAULT_POOL = [];

const STATUS_LABELS = {
  ready: "就绪",
  playing: "播放中",
  paused: "已暂停",
  stopped: "已停止",
  probing: "读取中",
  error: "不可用",
};

function getPrimaryAction(playbackStatus) {
  if (playbackStatus === "playing") {
    return { action: "pause", icon: <PauseOutlined />, label: "暂停" };
  }
  if (playbackStatus === "paused") {
    return { action: "resume", icon: <PlayCircleOutlined />, label: "继续" };
  }
  return { action: "play", icon: <PlayCircleOutlined />, label: "播放" };
}

function CycleRange({ label, minValue, maxValue, onChange }) {
  return (
    <div className="cycle-range-row">
      <span>{label}</span>
      <InputNumber
        aria-label={`${label}最小秒数`}
        min={1}
        max={60}
        size="small"
        value={minValue}
        onChange={(value) => onChange?.("min", value)}
      />
      <span aria-hidden="true">—</span>
      <InputNumber
        aria-label={`${label}最大秒数`}
        min={1}
        max={60}
        size="small"
        value={maxValue}
        onChange={(value) => onChange?.("max", value)}
      />
      <span>秒</span>
    </div>
  );
}

export function PlaybackColumn({
  pool = DEFAULT_POOL,
  currentIndex = 0,
  playbackStatus = "ready",
  progress = 0,
  currentTime = "00:00",
  duration = "00:00",
  volume = 72,
  muted = false,
  cycleRange = { videoMin: 8, videoMax: 15, audioMin: 8, audioMax: 15 },
  linkedCycles = false,
  localStatus = [],
  onImport,
  onPlaybackAction,
  onSeek,
  onVolumeChange,
  onMuteToggle,
  onPictureInPicture,
  onCycleRangeChange,
  onLinkedCyclesChange,
}) {
  const [poolDrawerOpen, setPoolDrawerOpen] = useState(false);
  const currentItem = pool[currentIndex];
  const primaryAction = getPrimaryAction(playbackStatus);
  const canPlay = pool.length > 0 && playbackStatus !== "probing";

  return (
    <aside className="playback-column" aria-label="播放池与播放控制">
      <section className="panel-section playback-pool-section" aria-labelledby="playback-pool-title">
        <div className="section-heading">
          <div>
            <h2 id="playback-pool-title">播放池</h2>
            <span>本地有序循环 · {pool.length}/100 项</span>
          </div>
          <Space size={6}>
            <Button size="small" icon={<UnorderedListOutlined />} disabled={!pool.length} onClick={() => setPoolDrawerOpen(true)}>管理</Button>
            <Button size="small" icon={<FileAddOutlined />} onClick={onImport}>导入视频</Button>
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
                  className={`playback-pool-item${isCurrent ? " is-current" : ""}`}
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
                    <Tag color={item.status === "error" ? "error" : isCurrent ? "success" : "default"}>
                      {isCurrent ? "当前" : STATUS_LABELS[item.status] || "就绪"}
                    </Tag>
                  </div>
                </List.Item>
              );
            }}
          />
        ) : (
          <Empty
            className="playback-pool-empty"
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description="尚未导入本地视频"
          >
            <Button type="primary" icon={<FileAddOutlined />} onClick={onImport}>
              选择 1–100 个视频
            </Button>
          </Empty>
        )}

        {pool.length ? <Button className="replace-pool-button" block icon={<FileAddOutlined />} onClick={onImport}>重新选择视频</Button> : null}

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
            icon={primaryAction.icon}
            disabled={!canPlay}
            onClick={() => onPlaybackAction?.(primaryAction.action)}
          >
            {primaryAction.label}
          </Button>
          <Button
            icon={<StopOutlined />}
            disabled={!canPlay || playbackStatus === "stopped"}
            onClick={() => onPlaybackAction?.("stop")}
          >
            停止
          </Button>
          <Tooltip title="在画中画窗口查看当前画面">
            <Button
              aria-label="打开画中画"
              icon={<PictureOutlined />}
              disabled={!canPlay}
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

      <section className="panel-section cycle-settings-section" aria-labelledby="cycle-settings-title">
        <div className="section-heading compact-heading">
          <h2 id="cycle-settings-title">周期范围</h2>
          <ReloadOutlined aria-hidden="true" />
        </div>
        <CycleRange
          label="视频周期"
          minValue={cycleRange.videoMin}
          maxValue={cycleRange.videoMax}
          onChange={(edge, value) => onCycleRangeChange?.(`video${edge === "min" ? "Min" : "Max"}`, value)}
        />
        <CycleRange
          label="声音周期"
          minValue={cycleRange.audioMin}
          maxValue={cycleRange.audioMax}
          onChange={(edge, value) => onCycleRangeChange?.(`audio${edge === "min" ? "Min" : "Max"}`, value)}
        />
        <div className="switch-setting-row">
          <span>声音与视频联动周期</span>
          <Switch checked={linkedCycles} onChange={onLinkedCyclesChange} />
        </div>
      </section>

      <section className="panel-section local-status-section" aria-labelledby="local-status-title">
        <div className="section-heading compact-heading">
          <h2 id="local-status-title">本地运行状态</h2>
          <Tag color="success">本机</Tag>
        </div>
        <dl className="local-status-list">
          {localStatus.map((item) => (
            <div key={item.label} className="local-status-item">
              <dt>{item.label}</dt>
              <dd className={item.tone ? `status-${item.tone}` : undefined}>{item.value}</dd>
            </div>
          ))}
        </dl>
      </section>

      <Drawer title={`播放池管理（共 ${pool.length} 项）`} width="min(560px, 100vw)" open={poolDrawerOpen} onClose={() => setPoolDrawerOpen(false)}>
        <AlertPoolPolicy />
        <List
          className="pool-management-list"
          dataSource={pool}
          renderItem={(item, index) => (
            <List.Item>
              <List.Item.Meta title={`${index + 1}. ${item.name}`} description={item.meta} />
              {index === currentIndex ? <Tag color="success">当前播放</Tag> : <Tag>按序等待</Tag>}
            </List.Item>
          )}
        />
      </Drawer>
    </aside>
  );
}

function AlertPoolPolicy() {
  return <p className="pool-policy-copy">播放池按导入顺序只读展示；本版本不追加、不重排、不删除，重新选择会整批原子替换。</p>;
}
