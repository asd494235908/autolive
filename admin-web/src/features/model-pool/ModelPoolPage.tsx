import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Alert,
  App,
  Button,
  Card,
  Empty,
  Form,
  Input,
  InputNumber,
  Modal,
  Select,
  Space,
  Table,
  Tag,
  Typography,
} from 'antd';
import { useMemo, useRef, useState } from 'react';
import { apiClient, ApiClientError, createRequestId } from '../../api/client';
import { StatusTag } from '../../components/StatusTag';
import { createModelPoolIdempotencyKeyManager } from './modelPoolIdempotency';
import type {
  CreateModelPoolAccountRequest,
  ModelPoolAccountEnvelope,
  ModelPoolAccountSummary,
  ModelPoolConnectivityTestResponse,
  ModelPoolResponse,
  ModelUsageListResponse,
  UpdateModelPoolAccountRequest,
} from '../../types/api';

export function ModelPoolPage() {
  const { message, modal } = App.useApp();
  const queryClient = useQueryClient();
  const [form] = Form.useForm<CreateModelPoolAccountRequest>();
  const [editForm] = Form.useForm<UpdateModelPoolAccountRequest>();
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [editingAccount, setEditingAccount] = useState<ModelPoolAccountSummary | null>(null);
  const [createError, setCreateError] = useState<string | null>(null);
  const [disablingAccountId, setDisablingAccountId] = useState<string | null>(null);
  const [testingAccountId, setTestingAccountId] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<ModelPoolConnectivityTestResponse | null>(null);
  const [usagePage, setUsagePage] = useState(1);
  const testIdempotencyKeys = useRef(new Map<string, string>());
  const [idempotencyKeyManager] = useState(() =>
    createModelPoolIdempotencyKeyManager(createRequestId),
  );

  const modelPoolQuery = useQuery({
    queryKey: ['admin-model-pool'],
    queryFn: () => apiClient.get<ModelPoolResponse>('/api/v1/admin/model-pool'),
  });

  const usageQuery = useQuery({
    queryKey: ['admin-model-usage', usagePage],
    queryFn: () =>
      apiClient.get<ModelUsageListResponse>('/api/v1/admin/model-usage', {
        query: { page: usagePage, page_size: 20 },
      }),
  });

  const createModelAccountMutation = useMutation({
    mutationFn: (values: CreateModelPoolAccountRequest) =>
      apiClient.post<ModelPoolAccountEnvelope>('/api/v1/admin/model-pool', {
        body: values,
        headers: {
          'Idempotency-Key': idempotencyKeyManager.getKey(values),
        },
      }),
    onSuccess: async (response) => {
      idempotencyKeyManager.clear();
      setCreateError(null);
      void message.success(
        `号池账号已创建：${response.account.provider} / ${response.account.model}（request_id：${response.request_id}）`,
      );
      setCreateOpen(false);
      form.resetFields();
      await queryClient.invalidateQueries({ queryKey: ['admin-model-pool'] });
    },
    onError: (error) => {
      const errorMessage =
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '创建号池账号失败';
      setCreateError(`${errorMessage}；再次点击“确定”会复用本次幂等键提交。`);
      void message.error(errorMessage);
    },
  });

  const disableModelAccountMutation = useMutation({
    mutationFn: (accountId: string) =>
      apiClient.post<ModelPoolAccountEnvelope>(
        `/api/v1/admin/model-pool/${accountId}/disable`,
        {
          headers: {
            'Idempotency-Key': createRequestId(),
          },
        },
      ),
    onSuccess: async (response) => {
      void message.success(
        `号池账号已禁用（request_id：${response.request_id}）`,
      );
      await queryClient.invalidateQueries({ queryKey: ['admin-model-pool'] });
    },
    onError: (error) => {
      const errorMessage =
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '禁用号池账号失败';
      void message.error(errorMessage);
    },
    onSettled: () => {
      setDisablingAccountId(null);
    },
  });

  const updateModelAccountMutation = useMutation({
    mutationFn: ({ accountId, values }: { accountId: string; values: UpdateModelPoolAccountRequest }) =>
      apiClient.request<ModelPoolAccountEnvelope>(`/api/v1/admin/model-pool/${accountId}`, {
        method: 'PATCH',
        body: values,
        headers: { 'Idempotency-Key': createRequestId() },
      }),
    onSuccess: async (response) => {
      void message.success(`号池账号已更新（request_id：${response.request_id}）`);
      setEditOpen(false);
      setEditingAccount(null);
      await queryClient.invalidateQueries({ queryKey: ['admin-model-pool'] });
    },
    onError: (error) => {
      void message.error(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '更新号池账号失败',
      );
    },
  });

  const testModelAccountMutation = useMutation({
    mutationFn: (accountId: string) => {
      const idempotencyKey = testIdempotencyKeys.current.get(accountId) ?? createRequestId();
      testIdempotencyKeys.current.set(accountId, idempotencyKey);
      return apiClient.post<ModelPoolConnectivityTestResponse>(
        `/api/v1/admin/model-pool/${accountId}/test`,
        {
          body: { timeout_seconds: 15 },
          headers: { 'Idempotency-Key': idempotencyKey },
          timeoutMs: 70_000,
        },
      );
    },
    onSuccess: async (response) => {
      testIdempotencyKeys.current.delete(response.account_id);
      setTestResult(response);
      void message.success(`连通性测试完成：${response.status}（${response.latency_ms} ms）`);
      await queryClient.invalidateQueries({ queryKey: ['admin-model-pool'] });
    },
    onError: (error) => {
      void message.error(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '号池连通性测试失败',
      );
    },
    onSettled: () => setTestingAccountId(null),
  });

  const columns = useMemo(
    () => [
      {
        title: '供应商',
        dataIndex: 'provider',
        key: 'provider',
      },
      {
        title: '模型',
        dataIndex: 'model',
        key: 'model',
      },
      {
        title: '代理地址',
        dataIndex: 'base_url',
        key: 'base_url',
        render: (value?: string) => value || '未配置',
      },
      {
        title: '状态',
        dataIndex: 'status',
        key: 'status',
        render: (status: ModelPoolAccountSummary['status']) => <StatusTag status={status} />,
      },
      {
        title: '优先级',
        dataIndex: 'priority',
        key: 'priority',
        render: (value?: number) => value ?? '未设置',
      },
      {
        title: '每日额度（预留）',
        dataIndex: 'daily_limit',
        key: 'daily_limit',
        render: (value?: number) => value ?? '未设置',
      },
      {
        title: '并发上限',
        dataIndex: 'concurrency_limit',
        key: 'concurrency_limit',
        render: (value?: number) => value ?? '未设置',
      },
      {
        title: '活动租约',
        dataIndex: 'active_leases',
        key: 'active_leases',
      },
      {
        title: '今日用量',
        key: 'daily_used_tokens',
        render: (_value: unknown, record: ModelPoolAccountSummary) =>
          `${record.daily_used_tokens}${record.daily_limit ? ` / ${record.daily_limit}` : ''}`,
      },
      {
        title: '密钥状态',
        dataIndex: 'secret_configured',
        key: 'secret_configured',
        render: (secretConfigured: boolean) =>
          secretConfigured ? <Tag color="success">已配置</Tag> : <Tag>未配置</Tag>,
      },
      {
        title: '操作',
        key: 'actions',
        render: (_value: unknown, record: ModelPoolAccountSummary) => (
          <Space>
            <Button
              type="link"
              loading={testingAccountId === record.id}
              disabled={testModelAccountMutation.isPending && testingAccountId !== record.id}
              onClick={() => {
                setTestingAccountId(record.id);
                testModelAccountMutation.mutate(record.id);
              }}
            >
              测试
            </Button>
            <Button
              type="link"
              onClick={() => {
                setEditingAccount(record);
                editForm.setFieldsValue({
                  base_url: record.base_url,
                  priority: record.priority ?? 0,
                  daily_limit: record.daily_limit ?? 0,
                  concurrency_limit: record.concurrency_limit ?? 1,
                  status: record.status,
                });
                setEditOpen(true);
              }}
            >
              编辑
            </Button>
            <Button
              danger
              type="link"
              disabled={record.status === 'disabled' || disableModelAccountMutation.isPending}
              loading={disablingAccountId === record.id}
              onClick={() => {
                modal.confirm({
                  title: '确认禁用号池账号？',
                  content: '禁用后不会再分配新的模型租约；如仍有活动租约，服务端会拒绝本次操作。',
                  okText: '确认禁用',
                  cancelText: '取消',
                  okButtonProps: { danger: true },
                  onOk: () => {
                    setDisablingAccountId(record.id);
                    return disableModelAccountMutation.mutateAsync(record.id);
                  },
                });
              }}
            >
              {record.status === 'disabled' ? '已禁用' : '禁用'}
            </Button>
          </Space>
        ),
      },
    ],
    [
      disableModelAccountMutation.isPending,
      disablingAccountId,
      editForm,
      modal,
      testModelAccountMutation.isPending,
      testingAccountId,
    ],
  );

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Space align="start" style={{ width: '100%', justifyContent: 'space-between' }}>
        <div>
          <Typography.Title level={2} style={{ margin: 0 }}>
            号池管理
          </Typography.Title>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
            对齐 `/api/v1/admin/model-pool` 的摘要、新增、编辑和禁用接口；前端只展示
            `secret_configured`，不会回显已保存密钥。
          </Typography.Paragraph>
        </div>

        <Button
          type="primary"
          onClick={() => {
            setCreateError(null);
            setCreateOpen(true);
          }}
        >
          新增号池账号
        </Button>
      </Space>

      {modelPoolQuery.isError ? (
        <Alert
          type="error"
          showIcon
          message="号池摘要加载失败"
          description={
            <Space direction="vertical" size="small">
              <Typography.Text>
                {modelPoolQuery.error instanceof ApiClientError
                  ? `${modelPoolQuery.error.message}${
                      modelPoolQuery.error.requestId
                        ? `（request_id：${modelPoolQuery.error.requestId}）`
                        : ''
                    }`
                  : '发生未知错误'}
              </Typography.Text>
              <Button onClick={() => void modelPoolQuery.refetch()}>重试</Button>
            </Space>
          }
        />
      ) : null}

      <Card>
        <Table<ModelPoolAccountSummary>
          rowKey="id"
          columns={columns}
          dataSource={modelPoolQuery.data?.accounts ?? []}
          loading={modelPoolQuery.isLoading}
          pagination={false}
          locale={{
            emptyText: modelPoolQuery.isLoading ? '加载中...' : <Empty description="暂无号池账号" />,
          }}
        />
        <Typography.Text type="secondary">
          最近请求 ID：{modelPoolQuery.data?.request_id ?? '暂无'}
        </Typography.Text>
      </Card>

      {testResult ? (
        <Alert
          type={testResult.status === 'succeeded' ? 'success' : 'warning'}
          showIcon
          closable
          onClose={() => setTestResult(null)}
          message={`最近测试：${testResult.provider} / ${testResult.model} · ${testResult.status}`}
          description={`${testResult.response_summary ?? '无响应摘要'}；延迟 ${testResult.latency_ms} ms；测试时间 ${testResult.tested_at}`}
        />
      ) : null}

      <Card title="模型调用摘要">
        {usageQuery.isError ? (
          <Alert
            type="error"
            showIcon
            message="模型调用摘要加载失败"
            description={
              <Button onClick={() => void usageQuery.refetch()}>重试</Button>
            }
            style={{ marginBottom: 16 }}
          />
        ) : null}
        <Table
          rowKey="id"
          loading={usageQuery.isLoading}
          dataSource={usageQuery.data?.items ?? []}
          pagination={{
            current: usagePage,
            pageSize: 20,
            total: usageQuery.data?.pagination.total ?? 0,
            showSizeChanger: false,
            onChange: (page) => setUsagePage(page),
          }}
          locale={{ emptyText: usageQuery.isLoading ? '加载中...' : <Empty description="暂无调用摘要" /> }}
          columns={[
            { title: '时间', dataIndex: 'created_at', key: 'created_at' },
            { title: '供应商', dataIndex: 'provider', key: 'provider' },
            { title: '模型', dataIndex: 'model', key: 'model' },
            { title: '调用 ID', dataIndex: 'client_call_id', key: 'client_call_id' },
            { title: 'Token', dataIndex: 'total_tokens', key: 'total_tokens' },
            { title: '延迟（ms）', dataIndex: 'latency_ms', key: 'latency_ms' },
            { title: '状态', dataIndex: 'status', key: 'status' },
          ]}
        />
      </Card>

      <Modal
        title="新增号池账号"
        open={createOpen}
        confirmLoading={createModelAccountMutation.isPending}
        onCancel={() => {
          if (!createModelAccountMutation.isPending) {
            idempotencyKeyManager.clear();
            setCreateError(null);
            form.resetFields();
            setCreateOpen(false);
          }
        }}
        onOk={() => {
          void form.validateFields().then((values) => {
            setCreateError(null);
            createModelAccountMutation.mutate(values);
          });
        }}
      >
        {createError ? (
          <Alert
            type="error"
            showIcon
            style={{ marginBottom: 16 }}
            message="创建失败，可重试"
            description={createError}
          />
        ) : null}
        <Alert
          type="info"
          showIcon
          style={{ marginBottom: 16 }}
          message="密钥只提交一次"
          description="新增成功后页面只显示“已配置”，不会回显 api_key；失败时再次点击“确定”会复用同一幂等键。"
        />
        <Form<CreateModelPoolAccountRequest>
          form={form}
          layout="vertical"
          initialValues={{
            provider: 'openai-compatible',
            model: 'rewrite-model',
            base_url: 'https://api.openai.com/v1',
            priority: 10,
            daily_limit: 1000,
            concurrency_limit: 2,
          }}
        >
          <Form.Item
            label="供应商"
            name="provider"
            rules={[{ required: true, message: '请输入供应商标识' }]}
          >
            <Input placeholder="openai-compatible" />
          </Form.Item>
          <Form.Item
            label="模型"
            name="model"
            rules={[{ required: true, message: '请输入模型标识' }]}
          >
            <Input placeholder="rewrite-model" />
          </Form.Item>
          <Form.Item
            label="OpenAI-compatible 代理地址"
            name="base_url"
            rules={[{ required: true, message: '请输入代理地址' }, { type: 'url', message: '请输入有效 URL' }]}
          >
            <Input placeholder="https://api.openai.com/v1" />
          </Form.Item>
          <Form.Item
            label="API Key"
            name="api_key"
            rules={[{ required: true, message: '请输入 API Key' }]}
          >
            <Input.Password placeholder="仅本次提交使用，不回显" autoComplete="new-password" />
          </Form.Item>
          <Form.Item
            label="优先级"
            name="priority"
            rules={[{ required: true, message: '请输入优先级' }]}
          >
            <InputNumber min={0} precision={0} style={{ width: '100%' }} />
          </Form.Item>
          <Form.Item
            label="每日额度（token/日，0 表示不限）"
            name="daily_limit"
            rules={[{ required: true, message: '请输入每日额度' }]}
          >
            <InputNumber min={0} precision={0} style={{ width: '100%' }} />
          </Form.Item>
          <Form.Item
            label="并发上限"
            name="concurrency_limit"
            rules={[{ required: true, message: '请输入并发上限' }]}
          >
            <InputNumber min={1} precision={0} style={{ width: '100%' }} />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        title={`编辑号池账号${editingAccount ? `：${editingAccount.provider} / ${editingAccount.model}` : ''}`}
        open={editOpen}
        confirmLoading={updateModelAccountMutation.isPending}
        onCancel={() => {
          if (!updateModelAccountMutation.isPending) {
            setEditOpen(false);
            setEditingAccount(null);
            editForm.resetFields();
          }
        }}
        onOk={() => {
          void editForm.validateFields().then((values) => {
            if (!editingAccount) {
              return;
            }
            updateModelAccountMutation.mutate({ accountId: editingAccount.id, values });
          });
        }}
      >
        <Alert
          type="info"
          showIcon
          style={{ marginBottom: 16 }}
          message="只编辑运行配置"
          description="此处不会读取或修改 API Key；禁用/冷却只影响新租约分配，服务端会保护活动租约和并发上限。"
        />
        <Form<UpdateModelPoolAccountRequest> form={editForm} layout="vertical">
          <Form.Item label="OpenAI-compatible 代理地址" name="base_url" rules={[{ type: 'url', message: '请输入有效 URL' }]}>
            <Input placeholder="https://api.openai.com/v1" />
          </Form.Item>
          <Form.Item label="状态" name="status" rules={[{ required: true, message: '请选择状态' }]}>
            <Select
              options={[
                { label: '可用', value: 'active' },
                { label: '冷却', value: 'cooldown' },
                { label: '额度耗尽', value: 'exhausted' },
                { label: '禁用', value: 'disabled' },
              ]}
            />
          </Form.Item>
          <Form.Item label="优先级" name="priority" rules={[{ required: true, message: '请输入优先级' }]}>
            <InputNumber min={0} precision={0} style={{ width: '100%' }} />
          </Form.Item>
          <Form.Item label="每日额度（token/日，0 表示不限）" name="daily_limit" rules={[{ required: true, message: '请输入每日额度' }]}>
            <InputNumber min={0} precision={0} style={{ width: '100%' }} />
          </Form.Item>
          <Form.Item label="并发上限" name="concurrency_limit" rules={[{ required: true, message: '请输入并发上限' }]}>
            <InputNumber min={1} precision={0} style={{ width: '100%' }} />
          </Form.Item>
        </Form>
      </Modal>
    </Space>
  );
}
