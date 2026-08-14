import { Tag } from 'antd';

const statusColorMap: Record<string, string> = {
  active: 'green',
  disabled: 'red',
  pending_activation: 'orange',
  revoked: 'red',
  used: 'blue',
  expired: 'default',
  cooldown: 'orange',
  exhausted: 'volcano',
  draft: 'default',
  authorized: 'blue',
  running: 'processing',
  completed: 'green',
  failed: 'red',
  cancelled: 'default',
  pending: 'default',
  succeeded: 'green'
};

const statusLabelMap: Record<string, string> = {
  active: '启用',
  disabled: '禁用',
  pending_activation: '待激活',
  revoked: '已撤销',
  used: '已使用',
  expired: '已过期',
  cooldown: '冷却中',
  exhausted: '已耗尽',
  draft: '草稿',
  authorized: '已授权',
  running: '运行中',
  completed: '已完成',
  failed: '失败',
  cancelled: '已取消',
  pending: '待处理',
  succeeded: '成功'
};

export function StatusTag({ status }: { status: string }) {
  return <Tag color={statusColorMap[status] ?? 'default'}>{statusLabelMap[status] ?? status}</Tag>;
}
