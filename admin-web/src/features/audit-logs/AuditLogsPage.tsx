import { useQuery } from '@tanstack/react-query';
import { Alert, Button, Card, Empty, Input, Select, Space, Table, Tag, Typography } from 'antd';
import { useState } from 'react';
import { apiClient, ApiClientError } from '../../api/client';
import type { AuditLog, AuditLogListResponse } from '../../types/api';

type AuditOutcome = AuditLog['outcome'] | '';
type AuditSort = 'created_at_desc' | 'created_at_asc';
type AuditTextFilterKey =
  | 'actor_user_id'
  | 'device_id'
  | 'action'
  | 'target_type'
  | 'error_code'
  | 'request_id'
  | 'created_after'
  | 'created_before';
type AuditFilters = {
  outcome: AuditOutcome;
  actor_user_id: string;
  device_id: string;
  action: string;
  target_type: string;
  error_code: string;
  request_id: string;
  created_after: string;
  created_before: string;
  sort: AuditSort;
};

const initialFilters: AuditFilters = {
  outcome: '',
  actor_user_id: '',
  device_id: '',
  action: '',
  target_type: '',
  error_code: '',
  request_id: '',
  created_after: '',
  created_before: '',
  sort: 'created_at_desc',
};

export function AuditLogsPage() {
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [filters, setFilters] = useState<AuditFilters>(initialFilters);
  const [draftFilters, setDraftFilters] = useState<AuditFilters>(initialFilters);
  const auditLogsQuery = useQuery({
    queryKey: ['audit-logs', page, pageSize, filters],
    queryFn: () =>
      apiClient.get<AuditLogListResponse>('/api/v1/admin/audit-logs', {
        query: {
          page,
          page_size: pageSize,
          outcome: filters.outcome || undefined,
          actor_user_id: filters.actor_user_id || undefined,
          device_id: filters.device_id || undefined,
          action: filters.action || undefined,
          target_type: filters.target_type || undefined,
          error_code: filters.error_code || undefined,
          request_id: filters.request_id || undefined,
          created_after: filters.created_after || undefined,
          created_before: filters.created_before || undefined,
          sort: filters.sort,
        }
      })
  });

  const applyTextFilter = (key: AuditTextFilterKey, value: string) => {
    const normalized = value.trim();
    setDraftFilters((current) => ({ ...current, [key]: normalized }));
    setFilters((current) => ({ ...current, [key]: normalized }));
    setPage(1);
  };

  const updateSelectFilter = <K extends 'outcome' | 'sort'>(key: K, value: AuditFilters[K]) => {
    setFilters((current) => ({ ...current, [key]: value }));
    setDraftFilters((current) => ({ ...current, [key]: value }));
    setPage(1);
  };

  const columns = [
    { title: '时间', dataIndex: 'created_at', key: 'created_at', render: (value: string) => new Date(value).toLocaleString('zh-CN') },
    { title: '操作者', dataIndex: 'actor_user_id', key: 'actor_user_id', render: (value?: string) => value || '系统' },
    { title: '设备', dataIndex: 'device_id', key: 'device_id', render: (value?: string) => value || '—' },
    { title: '动作', dataIndex: 'action', key: 'action' },
    { title: '目标类型', dataIndex: 'target_type', key: 'target_type' },
    { title: '结果', dataIndex: 'outcome', key: 'outcome', render: (value: AuditLog['outcome'], record: AuditLog) => <Tag color={value === 'success' ? 'success' : value === 'failure' ? 'error' : 'default'}>{value} / {record.status_code}</Tag> },
    { title: '目标 ID', dataIndex: 'target_id', key: 'target_id', render: (value?: string | null) => value || '—' },
    { title: '请求 ID', dataIndex: 'request_id', key: 'request_id', render: (value?: string) => value || '—' }
  ];

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Typography.Title level={2} style={{ margin: 0 }}>
        审计日志
      </Typography.Title>
      <Alert
        type={auditLogsQuery.isError ? 'error' : 'info'}
        showIcon
        message={auditLogsQuery.isError ? '审计日志加载失败' : '审计边界说明'}
        description={
          auditLogsQuery.isError ? (
            <Space direction="vertical" size="small">
              <Typography.Text>
                {auditLogsQuery.error instanceof ApiClientError
                  ? `${auditLogsQuery.error.message}${auditLogsQuery.error.requestId ? `（request_id：${auditLogsQuery.error.requestId}）` : ''}`
                  : '发生未知错误'}
              </Typography.Text>
              <Button onClick={() => void auditLogsQuery.refetch()}>重试</Button>
            </Space>
          ) : (
            '当前展示动作、目标、结果和 request_id；不读取请求体，不记录密码、Token、API Key 或明文激活码。'
          )
        }
      />
      <Card size="small" title="筛选与排序">
        <Space wrap>
          <Select<AuditOutcome>
            aria-label="审计结果"
            value={filters.outcome}
            style={{ width: 140 }}
            options={[
              { label: '全部结果', value: '' },
              { label: '成功', value: 'success' },
              { label: '失败', value: 'failure' },
              { label: '未知', value: 'unknown' },
            ]}
            onChange={(value) => updateSelectFilter('outcome', value)}
          />
          <Select<AuditSort>
            aria-label="审计排序"
            value={filters.sort}
            style={{ width: 180 }}
            options={[
              { label: '时间（新到旧）', value: 'created_at_desc' },
              { label: '时间（旧到新）', value: 'created_at_asc' },
            ]}
            onChange={(value) => updateSelectFilter('sort', value)}
          />
          {([
            ['actor_user_id', '操作者 ID'],
            ['device_id', '设备 ID'],
            ['action', '动作（精确）'],
            ['target_type', '目标类型'],
            ['error_code', '错误码'],
            ['request_id', '请求 ID'],
            ['created_after', '起始时间 RFC3339'],
            ['created_before', '结束时间 RFC3339'],
          ] as const).map(([key, placeholder]) => (
            <Input.Search
              key={key}
              allowClear
              enterButton="筛选"
              placeholder={placeholder}
              value={draftFilters[key]}
              style={{ width: 210 }}
              onChange={(event) => setDraftFilters((current) => ({ ...current, [key]: event.target.value }))}
              onSearch={(value) => applyTextFilter(key, value)}
              onClear={() => applyTextFilter(key, '')}
            />
          ))}
          <Button
            onClick={() => {
              setFilters(initialFilters);
              setDraftFilters(initialFilters);
              setPage(1);
            }}
          >
            重置
          </Button>
        </Space>
      </Card>
      <Card>
        <Table<AuditLog>
          rowKey="id"
          columns={columns}
          dataSource={auditLogsQuery.data?.items ?? []}
          loading={auditLogsQuery.isLoading}
          pagination={{
            current: page,
            pageSize,
            total: auditLogsQuery.data?.pagination.total ?? 0,
            showSizeChanger: true
          }}
          locale={{
            emptyText: auditLogsQuery.isLoading ? '加载中...' : <Empty description="暂无审计记录" />
          }}
          onChange={(pagination) => {
            const nextPageSize = pagination.pageSize ?? 20;
            setPageSize(nextPageSize);
            setPage(nextPageSize === pageSize ? pagination.current ?? 1 : 1);
          }}
        />
        <Typography.Text type="secondary">
          最近请求 ID：{auditLogsQuery.data?.request_id ?? '暂无'}
        </Typography.Text>
      </Card>
    </Space>
  );
}
