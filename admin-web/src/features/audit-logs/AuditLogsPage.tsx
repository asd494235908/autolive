import { useQuery } from '@tanstack/react-query';
import { Alert, Button, Card, Empty, Space, Table, Typography } from 'antd';
import { useState } from 'react';
import { apiClient, ApiClientError } from '../../api/client';
import type { AuditLog, AuditLogListResponse } from '../../types/api';

export function AuditLogsPage() {
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const auditLogsQuery = useQuery({
    queryKey: ['audit-logs', page, pageSize],
    queryFn: () =>
      apiClient.get<AuditLogListResponse>('/api/v1/admin/audit-logs', {
        query: { page, page_size: pageSize }
      })
  });

  const columns = [
    { title: '时间', dataIndex: 'created_at', key: 'created_at', render: (value: string) => new Date(value).toLocaleString('zh-CN') },
    { title: '操作者', dataIndex: 'actor_user_id', key: 'actor_user_id', render: (value?: string) => value || '系统' },
    { title: '设备', dataIndex: 'device_id', key: 'device_id', render: (value?: string) => value || '—' },
    { title: '动作', dataIndex: 'action', key: 'action' },
    { title: '目标类型', dataIndex: 'target_type', key: 'target_type' },
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
            '当前只展示动作、操作者、设备和 request_id，不读取请求体，不记录密码、Token、API Key 或明文激活码。'
          )
        }
      />
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
            setPage(pagination.current ?? 1);
            setPageSize(pagination.pageSize ?? 20);
          }}
        />
        <Typography.Text type="secondary">
          最近请求 ID：{auditLogsQuery.data?.request_id ?? '暂无'}
        </Typography.Text>
      </Card>
    </Space>
  );
}
