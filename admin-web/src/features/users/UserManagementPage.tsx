import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Alert,
  App,
  Button,
  Card,
  Empty,
  Form,
  Input,
  Modal,
  Select,
  Space,
  Table,
  Typography
} from 'antd';
import { useMemo, useState } from 'react';
import { apiClient, ApiClientError, createRequestId } from '../../api/client';
import { StatusTag } from '../../components/StatusTag';
import type { CreateUserRequest, UserEnvelope, UserListResponse, UserSummary } from '../../types/api';

export function UserManagementPage() {
  const { message, modal } = App.useApp();
  const queryClient = useQueryClient();
  const [form] = Form.useForm<CreateUserRequest>();
  const [createOpen, setCreateOpen] = useState(false);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);

  const usersQuery = useQuery({
    queryKey: ['admin-users', page, pageSize],
    queryFn: () =>
      apiClient.get<UserListResponse>('/api/v1/admin/users', {
        query: { page, page_size: pageSize }
      })
  });

  const createUserMutation = useMutation({
    mutationFn: (values: CreateUserRequest) =>
      apiClient.post<UserEnvelope>('/api/v1/admin/users', {
        body: values,
        headers: {
          'Idempotency-Key': createRequestId()
        }
      }),
    onSuccess: async (response) => {
      void message.success(`用户已创建：${response.user.username}（request_id：${response.request_id}）`);
      setCreateOpen(false);
      form.resetFields();
      await queryClient.invalidateQueries({ queryKey: ['admin-users'] });
    },
    onError: (error) => {
      if (error instanceof ApiClientError) {
        void message.error(
          `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
        );
        return;
      }

      void message.error('创建用户失败');
    }
  });

  const disableUserMutation = useMutation({
    mutationFn: (userId: string) =>
      apiClient.post<UserEnvelope>(`/api/v1/admin/users/${userId}/disable`, {
        headers: { 'Idempotency-Key': createRequestId() }
      }),
    onSuccess: async (response) => {
      void message.success(`用户已禁用：${response.user.username}`);
      await queryClient.invalidateQueries({ queryKey: ['admin-users'] });
    },
    onError: (error) => {
      void message.error(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '禁用用户失败'
      );
    }
  });

  const columns = useMemo(
    () => [
      {
        title: '用户名',
        dataIndex: 'username',
        key: 'username'
      },
      {
        title: '角色',
        dataIndex: 'role',
        key: 'role'
      },
      {
        title: '状态',
        dataIndex: 'status',
        key: 'status',
        render: (status: UserSummary['status']) => <StatusTag status={status} />
      },
      {
        title: '创建时间',
        dataIndex: 'created_at',
        key: 'created_at',
        render: (value: string) => new Date(value).toLocaleString('zh-CN')
      },
      {
        title: '操作',
        key: 'actions',
        render: (_: unknown, record: UserSummary) => (
          <Button
            danger
            disabled={record.status === 'disabled' || record.id === 'usr_local_admin'}
            loading={disableUserMutation.isPending}
            onClick={() => {
              modal.confirm({
                title: '确认禁用用户',
                content: `将禁用用户“${record.username}”，后续受保护操作会被拒绝。`,
                okText: '确认禁用',
                cancelText: '返回',
                onOk: () => disableUserMutation.mutateAsync(record.id)
              });
            }}
          >
            禁用
          </Button>
        )
      }
    ],
    [disableUserMutation, modal]
  );

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Space align="start" style={{ width: '100%', justifyContent: 'space-between' }}>
        <div>
          <Typography.Title level={2} style={{ margin: 0 }}>
            用户管理
          </Typography.Title>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
            对齐 `/api/v1/admin/users` 的列表、创建和禁用接口。
          </Typography.Paragraph>
        </div>

        <Button type="primary" onClick={() => setCreateOpen(true)}>
          创建用户
        </Button>
      </Space>

      {usersQuery.isError ? (
        <Alert
          type="error"
          showIcon
          message="用户列表加载失败"
          description={
            <Space direction="vertical" size="small">
              <Typography.Text>
                {usersQuery.error instanceof ApiClientError
                  ? `${usersQuery.error.message}${usersQuery.error.requestId ? `（request_id：${usersQuery.error.requestId}）` : ''}`
                  : '发生未知错误'}
              </Typography.Text>
              <Button onClick={() => void usersQuery.refetch()}>重试</Button>
            </Space>
          }
        />
      ) : null}

      <Card>
        <Table<UserSummary>
          rowKey="id"
          columns={columns}
          dataSource={usersQuery.data?.items ?? []}
          loading={usersQuery.isLoading}
          pagination={{
            current: page,
            pageSize,
            total: usersQuery.data?.pagination.total ?? 0,
            showSizeChanger: true
          }}
          locale={{
            emptyText: usersQuery.isLoading ? '加载中...' : <Empty description="暂无用户数据" />
          }}
          onChange={(pagination) => {
            setPage(pagination.current ?? 1);
            setPageSize(pagination.pageSize ?? 20);
          }}
        />
        <Typography.Text type="secondary">
          最近请求 ID：{usersQuery.data?.request_id ?? '暂无'}
        </Typography.Text>
      </Card>

      <Modal
        title="创建用户"
        open={createOpen}
        confirmLoading={createUserMutation.isPending}
        onCancel={() => {
          if (!createUserMutation.isPending) {
            setCreateOpen(false);
          }
        }}
        onOk={() => {
          void form.validateFields().then((values) => createUserMutation.mutate(values));
        }}
      >
        <Form<CreateUserRequest> form={form} layout="vertical" initialValues={{ role: 'user' }}>
          <Form.Item
            label="用户名"
            name="username"
            rules={[
              { required: true, message: '请输入用户名' },
              { min: 3, message: '用户名至少 3 个字符' }
            ]}
          >
            <Input placeholder="请输入用户名" />
          </Form.Item>
          <Form.Item
            label="密码"
            name="password"
            rules={[
              { required: true, message: '请输入密码' },
              { min: 8, message: '密码至少 8 个字符' }
            ]}
          >
            <Input.Password placeholder="请输入密码" />
          </Form.Item>
          <Form.Item label="角色" name="role" rules={[{ required: true, message: '请选择角色' }]}>
            <Select
              options={[
                { label: '普通用户', value: 'user' },
                { label: '管理员', value: 'admin' }
              ]}
            />
          </Form.Item>
        </Form>
      </Modal>
    </Space>
  );
}
