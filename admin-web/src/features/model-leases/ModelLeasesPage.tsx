import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Card, Descriptions, Empty, Input, Modal, Popconfirm, Select, Space, Table, Tag, Typography } from 'antd';
import { useState } from 'react';
import { apiClient, ApiClientError, createRequestId } from '../../api/client';
import { StatusTag } from '../../components/StatusTag';
import type { ModelLeaseAdminDetailResponse, ModelLeaseAdminSummary, ModelLeaseListResponse, ModelLeaseStatus, ReleaseModelLeaseResponse } from '../../types/api';

type ModelLeaseSort = 'expires_at_desc' | 'expires_at_asc' | 'status' | 'provider_model';
type TextFilterKey = 'provider' | 'model' | 'user_id' | 'device_id' | 'account_id';
type ModelLeaseFilters = {
  status: ModelLeaseStatus | '';
  provider: string;
  model: string;
  user_id: string;
  device_id: string;
  account_id: string;
  sort: ModelLeaseSort;
};

const initialFilters: ModelLeaseFilters = {
  status: '',
  provider: '',
  model: '',
  user_id: '',
  device_id: '',
  account_id: '',
  sort: 'expires_at_desc',
};

export function ModelLeasesPage() {
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [filters, setFilters] = useState<ModelLeaseFilters>(initialFilters);
  const [draftFilters, setDraftFilters] = useState<ModelLeaseFilters>(initialFilters);
  const [selectedLeaseId, setSelectedLeaseId] = useState<string | null>(null);
  const [reclaimReason, setReclaimReason] = useState('');
  const [reclaimIdempotencyKey, setReclaimIdempotencyKey] = useState<string | null>(null);
  const leasesQuery = useQuery({
    queryKey: ['admin-model-leases', page, pageSize, filters],
    queryFn: () =>
      apiClient.get<ModelLeaseListResponse>('/api/v1/admin/model-leases', {
        query: {
          page,
          page_size: pageSize,
          status: filters.status || undefined,
          provider: filters.provider || undefined,
          model: filters.model || undefined,
          user_id: filters.user_id || undefined,
          device_id: filters.device_id || undefined,
          account_id: filters.account_id || undefined,
          sort: filters.sort,
        }
      })
  });
  const detailQuery = useQuery({
    queryKey: ['admin-model-lease-detail', selectedLeaseId],
    queryFn: () => apiClient.get<ModelLeaseAdminDetailResponse>(`/api/v1/admin/model-leases/${encodeURIComponent(selectedLeaseId ?? '')}`),
    enabled: selectedLeaseId !== null,
  });
  const reclaimMutation = useMutation({
    mutationFn: async () => {
      if (!selectedLeaseId) {
        throw new Error('未选择租约');
      }
      return apiClient.post<ReleaseModelLeaseResponse>(`/api/v1/admin/model-leases/${encodeURIComponent(selectedLeaseId)}/reclaim`, {
        headers: { 'Idempotency-Key': reclaimIdempotencyKey ?? createRequestId() },
        body: { reason: reclaimReason.trim() || undefined },
      });
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['admin-model-leases'] });
      await queryClient.invalidateQueries({ queryKey: ['admin-model-lease-detail', selectedLeaseId] });
    },
  });

  const applyTextFilter = (key: TextFilterKey, value: string) => {
    const normalized = value.trim();
    setDraftFilters((current) => ({ ...current, [key]: normalized }));
    setFilters((current) => ({ ...current, [key]: normalized }));
    setPage(1);
  };

  const updateSelectFilter = <K extends 'status' | 'sort'>(key: K, value: ModelLeaseFilters[K]) => {
    setFilters((current) => ({ ...current, [key]: value }));
    setDraftFilters((current) => ({ ...current, [key]: value }));
    setPage(1);
  };

  const columns = [
    { title: '租约 ID', dataIndex: 'id', key: 'id' },
    { title: '账号 ID', dataIndex: 'account_id', key: 'account_id' },
    { title: '用户 ID', dataIndex: 'user_id', key: 'user_id' },
    { title: '设备 ID', dataIndex: 'device_id', key: 'device_id' },
    { title: '供应商 / 模型', key: 'model', render: (_: unknown, record: ModelLeaseAdminSummary) => `${record.provider} / ${record.model}` },
    { title: '用途', dataIndex: 'purpose', key: 'purpose' },
    { title: '状态', dataIndex: 'status', key: 'status', render: (value: ModelLeaseAdminSummary['status']) => <StatusTag status={value} /> },
    { title: '到期时间', dataIndex: 'expires_at', key: 'expires_at', render: (value: string) => new Date(value).toLocaleString('zh-CN') },
    { title: '并发上限', dataIndex: 'concurrency_limit', key: 'concurrency_limit' },
    {
      title: '操作',
      key: 'actions',
      render: (_: unknown, record: ModelLeaseAdminSummary) => (
        <Button type="link" onClick={() => { setSelectedLeaseId(record.id); setReclaimReason(''); setReclaimIdempotencyKey(createRequestId()); reclaimMutation.reset(); }}>
          详情
        </Button>
      ),
    },
  ];

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Typography.Title level={2} style={{ margin: 0 }}>
        模型租约
      </Typography.Title>
      <Alert
        type={leasesQuery.isError ? 'error' : 'info'}
        showIcon
        message={leasesQuery.isError ? '模型租约加载失败' : '租约生命周期摘要'}
        description={
          leasesQuery.isError ? (
            <Space direction="vertical" size="small">
              <Typography.Text>
                {leasesQuery.error instanceof ApiClientError
                  ? `${leasesQuery.error.message}${leasesQuery.error.requestId ? `（request_id：${leasesQuery.error.requestId}）` : ''}`
                  : '发生未知错误'}
              </Typography.Text>
              <Button onClick={() => void leasesQuery.refetch()}>重试</Button>
            </Space>
          ) : (
            '只展示租约归属、状态、到期时间和并发上限；不会返回租约凭证、API Key 或模型正文。'
          )
        }
      />
      <Card size="small" title="筛选与排序">
        <Space wrap>
          <Select<ModelLeaseStatus | ''>
            aria-label="租约状态"
            value={filters.status}
            style={{ width: 140 }}
            options={[
              { label: '全部状态', value: '' },
              { label: '活动', value: 'active' },
              { label: '已释放', value: 'released' },
              { label: '已过期', value: 'expired' },
            ]}
            onChange={(value) => updateSelectFilter('status', value)}
          />
          <Select<ModelLeaseSort>
            aria-label="租约排序"
            value={filters.sort}
            style={{ width: 180 }}
            options={[
              { label: '到期时间（新到旧）', value: 'expires_at_desc' },
              { label: '到期时间（旧到新）', value: 'expires_at_asc' },
              { label: '状态', value: 'status' },
              { label: '供应商 / 模型', value: 'provider_model' },
            ]}
            onChange={(value) => updateSelectFilter('sort', value)}
          />
          {([
            ['provider', '供应商'],
            ['model', '模型'],
            ['user_id', '用户 ID'],
            ['device_id', '设备 ID'],
            ['account_id', '账号 ID'],
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
        <Table<ModelLeaseAdminSummary>
          rowKey="id"
          columns={columns}
          dataSource={leasesQuery.data?.items ?? []}
          loading={leasesQuery.isLoading}
          pagination={{
            current: page,
            pageSize,
            total: leasesQuery.data?.pagination.total ?? 0,
            showSizeChanger: true
          }}
          locale={{
            emptyText: leasesQuery.isLoading ? '加载中...' : <Empty description="暂无模型租约" />
          }}
          onChange={(pagination) => {
            const nextPageSize = pagination.pageSize ?? 20;
            setPageSize(nextPageSize);
            setPage(nextPageSize === pageSize ? pagination.current ?? 1 : 1);
          }}
        />
        <Typography.Text type="secondary">
          最近请求 ID：{leasesQuery.data?.request_id ?? '暂无'}
        </Typography.Text>
      </Card>
      <Modal
        title="模型租约详情"
        open={selectedLeaseId !== null}
        onCancel={() => { setSelectedLeaseId(null); setReclaimIdempotencyKey(null); reclaimMutation.reset(); }}
        footer={null}
        destroyOnClose
      >
        {detailQuery.isLoading ? <Typography.Text>加载中...</Typography.Text> : null}
        {detailQuery.isError ? (
          <Alert
            type="error"
            message="租约详情加载失败"
            description={detailQuery.error instanceof ApiClientError ? detailQuery.error.message : '发生未知错误'}
          />
        ) : null}
        {detailQuery.data?.lease ? (
          <Space direction="vertical" size="middle" style={{ width: '100%' }}>
            <Descriptions bordered size="small" column={1}>
              <Descriptions.Item label="租约 ID">{detailQuery.data.lease.id}</Descriptions.Item>
              <Descriptions.Item label="账号 / 用户 / 设备">{detailQuery.data.lease.account_id} / {detailQuery.data.lease.user_id} / {detailQuery.data.lease.device_id}</Descriptions.Item>
              <Descriptions.Item label="供应商 / 模型">{detailQuery.data.lease.provider} / {detailQuery.data.lease.model}</Descriptions.Item>
              <Descriptions.Item label="状态"><Tag>{detailQuery.data.lease.status}</Tag></Descriptions.Item>
              <Descriptions.Item label="创建时间">{detailQuery.data.lease.created_at ? new Date(detailQuery.data.lease.created_at).toLocaleString('zh-CN') : '—'}</Descriptions.Item>
              <Descriptions.Item label="到期时间">{new Date(detailQuery.data.lease.expires_at).toLocaleString('zh-CN')}</Descriptions.Item>
              <Descriptions.Item label="回收时间">{detailQuery.data.lease.released_at ? new Date(detailQuery.data.lease.released_at).toLocaleString('zh-CN') : '—'}</Descriptions.Item>
              <Descriptions.Item label="用途 / 并发上限">{detailQuery.data.lease.purpose} / {detailQuery.data.lease.concurrency_limit}</Descriptions.Item>
            </Descriptions>
            {detailQuery.data.lease.status === 'active' ? (
              <>
                <Input.TextArea
                  aria-label="回收原因"
                  maxLength={255}
                  showCount
                  placeholder="可选：填写回收原因"
                  value={reclaimReason}
                  onChange={(event) => {
                    if (reclaimMutation.isError) {
                      reclaimMutation.reset();
                      setReclaimIdempotencyKey(createRequestId());
                    }
                    setReclaimReason(event.target.value);
                  }}
                  autoSize={{ minRows: 2, maxRows: 4 }}
                />
                <Popconfirm
                  title="确认回收该模型租约？"
                  description="回收后客户端不能继续使用该租约。"
                  okText="确认回收"
                  cancelText="取消"
                  onConfirm={() => reclaimMutation.mutate()}
                >
                  <Button danger loading={reclaimMutation.isPending}>
                    管理员回收租约
                  </Button>
                </Popconfirm>
              </>
            ) : <Typography.Text type="secondary">租约已处于终态，无需再次回收。</Typography.Text>}
            {reclaimMutation.isError ? <Alert type="error" message="租约回收失败" description={reclaimMutation.error instanceof ApiClientError ? reclaimMutation.error.message : '发生未知错误'} /> : null}
            {reclaimMutation.isSuccess ? <Alert type="success" message="租约已回收" description={`request_id：${reclaimMutation.data.request_id}`} /> : null}
          </Space>
        ) : null}
      </Modal>
    </Space>
  );
}
