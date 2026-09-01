import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { Button, Card, Input, InputNumber, Space, Tag, Typography } from 'antd';
import { useEffect, useMemo, useState } from 'react';

type DouyinLiveProbeStatus = {
  running: boolean;
  state: string;
  lastEvent: string | null;
  eventHistory: string[];
  qrPath: string | null;
  roomResolved: boolean;
  chatReceived: boolean;
  replyAttempted: boolean;
  selfEchoFiltered: boolean;
  error: string | null;
};

type DouyinLiveProbeRequest = {
  upstreamRoot: string;
  roomId: string;
  replies: string[];
  timeoutSec: number;
};

const IDLE_STATUS: DouyinLiveProbeStatus = {
  running: false,
  state: 'idle',
  lastEvent: null,
  eventHistory: [],
  qrPath: null,
  roomResolved: false,
  chatReceived: false,
  replyAttempted: false,
  selfEchoFiltered: false,
  error: null,
};

const STATE_LABELS: Record<string, string> = {
  idle: '未启动',
  starting: '启动中',
  waiting_qr: '等待扫码',
  logged_in: '已登录，等待进入直播间',
  room_resolved: '直播间已解析',
  listening: '监听弹幕中',
  chat_received: '已读取一条外部弹幕',
  reply_attempted: '已尝试回复一条',
  self_echo_filtered: '已过滤自回显',
  passed: '最小链路通过',
  inconclusive: '证据不足',
  failed: '失败',
  stopped: '已停止',
};

function parseReplies(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function DouyinLivePanel() {
  const [upstreamRoot, setUpstreamRoot] = useState('');
  const [roomId, setRoomId] = useState('');
  const [replyText, setReplyText] = useState('GpAutoLive探针A\nGpAutoLive探针B');
  const [timeoutSec, setTimeoutSec] = useState(300);
  const [status, setStatus] = useState<DouyinLiveProbeStatus>(IDLE_STATUS);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const next = await invoke<DouyinLiveProbeStatus>('get_douyin_live_probe_status');
      setStatus(next);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '读取抖音探针状态失败');
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    if (!status.running) return undefined;
    const timer = window.setInterval(() => void refresh(), 1_000);
    return () => window.clearInterval(timer);
  }, [status.running]);

  const qrSource = useMemo(
    () => (status.qrPath ? convertFileSrc(status.qrPath) : null),
    [status.qrPath],
  );
  const stateLabel = STATE_LABELS[status.state] ?? status.state;

  const start = async () => {
    setError(null);
    const replies = parseReplies(replyText);
    if (!upstreamRoot.trim() || !roomId.trim() || replies.length === 0) {
      setError('请填写 Douyin_Spider 路径、直播间号和至少一条回复候选');
      return;
    }
    const request: DouyinLiveProbeRequest = {
      upstreamRoot: upstreamRoot.trim(),
      roomId: roomId.trim(),
      replies,
      timeoutSec,
    };
    try {
      const next = await invoke<DouyinLiveProbeStatus>('start_douyin_live_probe', { request });
      setStatus(next);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '启动抖音探针失败');
    }
  };

  const stop = async () => {
    setError(null);
    try {
      const next = await invoke<DouyinLiveProbeStatus>('stop_douyin_live_probe');
      setStatus(next);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '停止抖音探针失败');
    }
  };

  return (
    <Card
      size="small"
      title="抖音直播弹幕（M1 链路探针）"
      extra={<Tag color={status.state === 'passed' ? 'success' : status.running ? 'processing' : 'default'}>{stateLabel}</Tag>}
      style={{ marginBottom: 12 }}
    >
      <Typography.Paragraph type="secondary" style={{ marginBottom: 10 }}>
        仅用于自有测试直播间：扫码登录、读取一条外部弹幕、随机回复一条并过滤自己的回显；不调用模型、不经过 Go。
      </Typography.Paragraph>
      <Space direction="vertical" style={{ width: '100%' }} size={8}>
        <Input
          aria-label="Douyin_Spider 路径"
          placeholder="Douyin_Spider 本地目录，例如 C:\\src\\Douyin_Spider"
          value={upstreamRoot}
          onChange={(event) => setUpstreamRoot(event.target.value)}
          disabled={status.running}
        />
        <Input
          aria-label="自有测试直播间号"
          placeholder="自有测试直播间号或 https://live.douyin.com/<id>"
          value={roomId}
          onChange={(event) => setRoomId(event.target.value)}
          disabled={status.running}
        />
        <Input.TextArea
          aria-label="回复候选"
          rows={2}
          placeholder="每行一条候选，发送前随机选择"
          value={replyText}
          onChange={(event) => setReplyText(event.target.value)}
          disabled={status.running}
        />
        <Space wrap>
          <InputNumber
            aria-label="探针超时秒数"
            min={30}
            max={900}
            value={timeoutSec}
            onChange={(value) => setTimeoutSec(value ?? 300)}
            addonAfter="秒"
            disabled={status.running}
          />
          <Button type="primary" onClick={() => void start()} disabled={status.running}>开始扫码探针</Button>
          <Button onClick={() => void stop()} disabled={!status.running}>停止</Button>
          <Button onClick={() => void refresh()}>刷新状态</Button>
        </Space>
        {qrSource ? (
          <div>
            <Typography.Text strong>请用抖音 App 扫描二维码并确认</Typography.Text>
            <div style={{ marginTop: 8 }}>
              <img src={qrSource} alt="抖音扫码登录二维码" width={220} height={220} style={{ imageRendering: 'pixelated' }} />
            </div>
          </div>
        ) : null}
        <Space wrap size={[6, 4]}>
          <Tag color={status.roomResolved ? 'success' : 'default'}>直播间解析 {status.roomResolved ? '完成' : '等待'}</Tag>
          <Tag color={status.chatReceived ? 'success' : 'default'}>弹幕读取 {status.chatReceived ? '完成' : '等待'}</Tag>
          <Tag color={status.replyAttempted ? 'success' : 'default'}>回复尝试 {status.replyAttempted ? '完成' : '等待'}</Tag>
          <Tag color={status.selfEchoFiltered ? 'success' : 'default'}>自回显 {status.selfEchoFiltered ? '已过滤' : '等待'}</Tag>
        </Space>
        {status.lastEvent ? <Typography.Text type="secondary">最近事件：{status.lastEvent}</Typography.Text> : null}
        {error || status.error ? <Typography.Text type="danger">{error ?? status.error}</Typography.Text> : null}
      </Space>
    </Card>
  );
}

