import { Progress, Space, Tag, Typography } from 'antd';
import type { CSSProperties, ReactNode } from 'react';

import { CompactNumberField } from './compact-number-field';

type CycleRange = { minMs: number; maxMs: number };

function formatCycleSeconds(milliseconds: number) {
  const seconds = milliseconds / 1000;
  if (seconds >= 60) {
    const minutes = Math.floor(seconds / 60);
    const remainder = Math.round(seconds % 60);
    return remainder === 0 ? `${minutes} 分钟` : `${minutes}分${String(remainder).padStart(2, '0')}秒`;
  }
  return `${Number.isInteger(seconds) ? String(seconds) : seconds.toFixed(1)} 秒`;
}

export function MediaCycleCard({
  title,
  icon,
  range,
  changes,
  progress,
  status,
  statusColor,
  accent,
  editable = false,
  onRangeChange,
  footer,
}: {
  title: string;
  icon: ReactNode;
  range: CycleRange;
  changes: number;
  progress: number;
  status: string;
  statusColor?: string;
  accent: string;
  editable?: boolean;
  onRangeChange?: (range: CycleRange) => void;
  footer?: ReactNode;
}) {
  return (
    <section className="desktop-cycle-card" style={{ '--desktop-cycle-accent': accent } as CSSProperties} aria-label={`${title}状态`}>
      <div className="desktop-cycle-heading">
        <Space size={6}>{icon}<Typography.Text strong>{title}</Typography.Text></Space>
        <Tag color={statusColor}>{status}</Tag>
      </div>
      <div className="desktop-cycle-meta">
        <div className="desktop-cycle-range">
          <Typography.Text className="desktop-muted">范围</Typography.Text>
          {editable ? (
            <>
              <CompactNumberField ariaLabel={`${title}最小秒`} min={1} max={60} size="small" value={range.minMs / 1000} onChange={(value) => typeof value === 'number' && onRangeChange?.({ minMs: value * 1000, maxMs: range.maxMs })} />
              <Typography.Text className="desktop-muted">—</Typography.Text>
              <CompactNumberField ariaLabel={`${title}最大秒`} min={1} max={60} size="small" value={range.maxMs / 1000} onChange={(value) => typeof value === 'number' && onRangeChange?.({ minMs: range.minMs, maxMs: value * 1000 })} />
              <Typography.Text className="desktop-muted">秒</Typography.Text>
            </>
          ) : (
            <Typography.Text className="desktop-muted">{formatCycleSeconds(range.minMs)}–{formatCycleSeconds(range.maxMs)}</Typography.Text>
          )}
        </div>
        <Typography.Text>已变化 {changes} 次</Typography.Text>
      </div>
      <Progress aria-label={`${title}下一计划进度`} percent={Math.min(100, Math.max(0, progress))} showInfo={false} size={["100%", 4]} strokeColor={accent} style={{ margin: 0 }} />
      {footer}
    </section>
  );
}
