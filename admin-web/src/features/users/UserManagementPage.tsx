import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Alert,
  App,
  Button,
  Card,
  Descriptions,
  Drawer,
  Empty,
  Form,
  Input,
  InputNumber,
  Modal,
  Result,
  Select,
  Space,
  Table,
  Typography
} from 'antd';
import { useEffect, useMemo, useState } from 'react';
import { apiClient, ApiClientError, createRequestId } from '../../api/client';
import { StatusTag } from '../../components/StatusTag';
import { useAdminAuthorization } from '../admin-rbac/useAdminAuthorization';
import type {
  CreateUserRequest,
  ResetUserPasswordRequest,
  UpdateUserRequest,
  DeviceListResponse,
  DeviceSummary,
  UpdateUserAuthorizationRequest,
  UserAuthorizationPolicyResponse,
  UserAuthorizationSummaryResponse,
  UserEnvelope,
  UserListResponse,
  UserSummary
} from '../../types/api';

export function UserManagementPage() {
  const authorization = useAdminAuthorization();
  const canManageUsers = authorization.can('users.manage');
  const { message, modal } = App.useApp();
  const queryClient = useQueryClient();
  const [form] = Form.useForm<CreateUserRequest>();
  const [editForm] = Form.useForm<UpdateUserRequest>();
  const [resetForm] = Form.useForm<ResetUserPasswordRequest>();
  const [authorizationForm] = Form.useForm<UpdateUserAuthorizationRequest>();
  const [createOpen, setCreateOpen] = useState(false);
  const [editingUser, setEditingUser] = useState<UserSummary | null>(null);
  const [resettingUser, setResettingUser] = useState<UserSummary | null>(null);
  const [devicesUser, setDevicesUser] = useState<UserSummary | null>(null);
  const [devicePage, setDevicePage] = useState(1);
  const [devicePageSize, setDevicePageSize] = useState(20);
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

  const userDevicesQuery = useQuery({
    queryKey: ['admin-user-devices', devicesUser?.id, devicePage, devicePageSize],
    queryFn: () =>
      apiClient.get<DeviceListResponse>(`/api/v1/admin/users/${devicesUser?.id}/devices`, {
        query: { page: devicePage, page_size: devicePageSize }
      }),
    enabled: devicesUser !== null
  });

  const userAuthorizationQuery = useQuery({
    queryKey: ['admin-user-authorization-summary', devicesUser?.id],
    queryFn: () =>
      apiClient.get<UserAuthorizationSummaryResponse>(
        `/api/v1/admin/users/${devicesUser?.id}/authorization-summary`
      ),
    enabled: devicesUser !== null
  });

  useEffect(() => {
    const summary = userAuthorizationQuery.data?.summary;
    if (!summary) {
      return;
    }
    authorizationForm.setFieldsValue({
      allowed_models: summary.allowed_models,
      daily_token_limit: summary.daily_token_limit
    });
  }, [authorizationForm, userAuthorizationQuery.data]);

  const updateAuthorizationMutation = useMutation({
    mutationFn: ({ userId, values }: { userId: string; values: UpdateUserAuthorizationRequest }) =>
      apiClient.request<UserAuthorizationPolicyResponse>(`/api/v1/admin/users/${userId}/authorization`, {
        method: 'PATCH',
        body: values,
        headers: { 'Idempotency-Key': createRequestId() }
      }),
    onSuccess: async (response) => {
      void message.success(`用户授权策略已更新（request_id：${response.request_id}）`);
      await queryClient.invalidateQueries({ queryKey: ['admin-user-authorization-summary'] });
    },
    onError: (error) => {
      void message.error(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（${error.requestId}）` : ''}`
          : '更新用户授权策略失败'
      );
    }
  });

  const updateUserMutation = useMutation({
    mutationFn: ({ userId, values }: { userId: string; values: UpdateUserRequest }) =>
      apiClient.request<UserEnvelope>(`/api/v1/admin/users/${userId}`, {
        method: 'PATCH',
        body: values,
        headers: { 'Idempotency-Key': createRequestId() }
      }),
    onSuccess: async (response) => {
      void message.success(`用户已更新：${response.user.username}`);
      setEditingUser(null);
      await queryClient.invalidateQueries({ queryKey: ['admin-users'] });
    },
    onError: (error) => {
      void message.error(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '更新用户失败'
      );
    }
  });

  const resetPasswordMutation = useMutation({
    mutationFn: ({ userId, values }: { userId: string; values: ResetUserPasswordRequest }) =>
      apiClient.post<UserEnvelope>(`/api/v1/admin/users/${userId}/reset-password`, {
        body: values,
        headers: { 'Idempotency-Key': createRequestId() }
      }),
    onSuccess: async (response) => {
      void message.success(`密码已重置：${response.user.username}`);
      setResettingUser(null);
      resetForm.resetFields();
      await queryClient.invalidateQueries({ queryKey: ['admin-users'] });
    },
    onError: (error) => {
      void message.error(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '重置密码失败'
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
          <Space>
            <Button
              disabled={!canManageUsers || record.id === 'usr_local_admin'}
              onClick={() => {
                setEditingUser(record);
                editForm.setFieldsValue({
                  username: record.username,
                  role: record.role,
                  status: record.status
                });
              }}
            >
              编辑
            </Button>
            <Button
              disabled={!canManageUsers || record.id === 'usr_local_admin'}
              onClick={() => {
                setResettingUser(record);
                resetForm.resetFields();
              }}
            >
              重置密码
            </Button>
            <Button
              onClick={() => {
                setDevicePage(1);
                setDevicesUser(record);
              }}
            >
              设备
            </Button>
            <Button
              danger
              disabled={!canManageUsers || record.status === 'disabled' || record.id === 'usr_local_admin'}
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
          </Space>
        )
      }
    ],
    [disableUserMutation, editForm, modal, resetForm]
  );

  const deviceColumns = useMemo(
    () => [
      { title: '设备名', dataIndex: 'device_name', key: 'device_name' },
      { title: '平台', dataIndex: 'platform', key: 'platform' },
      {
        title: '状态',
        dataIndex: 'status',
        key: 'status',
        render: (status: DeviceSummary['status']) => <StatusTag status={status} />
      },
      {
        title: '在线',
        dataIndex: 'online',
        key: 'online',
        render: (online: boolean) => (online ? '在线' : '离线')
      },
      { title: '当前播放', dataIndex: 'current_media_name', key: 'current_media_name' }
    ],
    []
  );

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Space align="start" style={{ width: '100%', justifyContent: 'space-between' }}>
        <div>
          <Typography.Title level={2} style={{ margin: 0 }}>
            用户管理
          </Typography.Title>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
            对齐用户列表、创建、编辑、密码重置、禁用，以及模型授权与服务端记录用量门禁。
          </Typography.Paragraph>
        </div>

        <Button type="primary" disabled={!canManageUsers} onClick={() => setCreateOpen(true)}>
          创建用户
        </Button>
      </Space>

      {usersQuery.isError && usersQuery.error instanceof ApiClientError && usersQuery.error.status === 403 ? (
        <Result
          status="403"
          title="无权读取用户列表"
          subTitle="服务端拒绝了当前会话的用户读取请求，请刷新权限后重试。"
          extra={
            <Button
              type="primary"
              onClick={() => {
                void authorization.refresh();
                void usersQuery.refetch();
              }}
            >
              重新验证权限
            </Button>
          }
        />
      ) : null}

      {usersQuery.isError && (!(usersQuery.error instanceof ApiClientError) || usersQuery.error.status !== 403) ? (
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

      <Drawer
        title={devicesUser ? `用户授权与设备：${devicesUser.username}` : '用户授权与设备'}
        open={devicesUser !== null}
        width={720}
        onClose={() => setDevicesUser(null)}
      >
        {userAuthorizationQuery.isError && userAuthorizationQuery.error instanceof ApiClientError && userAuthorizationQuery.error.status === 403 ? (
          <Result status="403" title="无权读取用户授权摘要" subTitle="当前会话缺少 users.read 权限。" />
        ) : null}
        {userAuthorizationQuery.isError && (!(userAuthorizationQuery.error instanceof ApiClientError) || userAuthorizationQuery.error.status !== 403) ? (
          <Alert
            type="error"
            showIcon
            message="用户授权摘要加载失败"
            description={
              userAuthorizationQuery.error instanceof ApiClientError
                ? `${userAuthorizationQuery.error.message}${userAuthorizationQuery.error.requestId ? `（request_id：${userAuthorizationQuery.error.requestId}）` : ''}`
                : '发生未知错误'
            }
          />
        ) : (
          <Card size="small" title="授权与软额度摘要" loading={userAuthorizationQuery.isLoading} style={{ marginBottom: 16 }}>
            {userAuthorizationQuery.data ? (
              <Descriptions column={2} size="small">
                <Descriptions.Item label="设备总数">{userAuthorizationQuery.data.summary.device_count}</Descriptions.Item>
                <Descriptions.Item label="活动设备">{userAuthorizationQuery.data.summary.active_device_count}</Descriptions.Item>
                <Descriptions.Item label="活动租约">{userAuthorizationQuery.data.summary.active_lease_count}</Descriptions.Item>
                <Descriptions.Item label="活动模型账号">{userAuthorizationQuery.data.summary.active_account_count}</Descriptions.Item>
                <Descriptions.Item label="今日已用 Token">{userAuthorizationQuery.data.summary.daily_used_tokens}</Descriptions.Item>
                <Descriptions.Item label="额度依据">客户端自报软额度</Descriptions.Item>
                <Descriptions.Item label="模型授权">
                  {userAuthorizationQuery.data.summary.allowed_models.length > 0
                    ? userAuthorizationQuery.data.summary.allowed_models.join('、')
                    : '全部已登记模型'}
                </Descriptions.Item>
                <Descriptions.Item label="记录用量门禁">
                  {userAuthorizationQuery.data.summary.daily_token_limit > 0
                    ? `${userAuthorizationQuery.data.summary.daily_token_limit} Token/日`
                    : '未配置'}
                </Descriptions.Item>
                <Descriptions.Item label="供应商硬额度">未配置</Descriptions.Item>
                <Descriptions.Item label="统计时间">{new Date(userAuthorizationQuery.data.summary.as_of).toLocaleString('zh-CN')}</Descriptions.Item>
              </Descriptions>
            ) : null}
          </Card>
        )}
        <Card size="small" title="编辑模型授权与记录用量门禁" style={{ marginBottom: 16 }}>
          <Form<UpdateUserAuthorizationRequest>
            form={authorizationForm}
            layout="vertical"
            onFinish={(values) => {
              if (devicesUser) {
                updateAuthorizationMutation.mutate({ userId: devicesUser.id, values });
              }
            }}
          >
            <Form.Item
              label="允许的模型（provider/model）"
              name="allowed_models"
              rules={[{ type: 'array', max: 100, message: '最多配置 100 个模型' }]}
              extra="留空表示允许全部已登记模型；当前仅控制模型租约申请。"
            >
              <Select mode="tags" tokenSeparators={[',']} placeholder="例如 openai-compatible/gpt-4o-mini" />
            </Form.Item>
            <Form.Item
              label="每日服务端记录用量上限（Token）"
              name="daily_token_limit"
              rules={[{ required: true, type: 'number', min: 0, max: 1000000000, message: '请输入 0～1,000,000,000' }]}
              extra="只依据服务端已接收记录，不是供应商权威账单或未来用量预占。"
            >
              <InputNumber min={0} max={1000000000} style={{ width: '100%' }} />
            </Form.Item>
            <Button
              type="primary"
              htmlType="submit"
              loading={updateAuthorizationMutation.isPending}
              disabled={!canManageUsers}
            >
              保存授权策略
            </Button>
          </Form>
        </Card>
        {userDevicesQuery.isError && userDevicesQuery.error instanceof ApiClientError && userDevicesQuery.error.status === 403 ? (
          <Result status="403" title="无权读取用户设备列表" subTitle="当前会话缺少 users.read 权限。" />
        ) : null}
        {userDevicesQuery.isError && (!(userDevicesQuery.error instanceof ApiClientError) || userDevicesQuery.error.status !== 403) ? (
          <Alert
            type="error"
            showIcon
            message="用户设备加载失败"
            description={
              userDevicesQuery.error instanceof ApiClientError
                ? `${userDevicesQuery.error.message}${userDevicesQuery.error.requestId ? `（request_id：${userDevicesQuery.error.requestId}）` : ''}`
                : '发生未知错误'
            }
          />
        ) : (
          <Table<DeviceSummary>
            rowKey="id"
            size="small"
            columns={deviceColumns}
            dataSource={userDevicesQuery.data?.items ?? []}
            loading={userDevicesQuery.isLoading}
            pagination={{
              current: devicePage,
              pageSize: devicePageSize,
              total: userDevicesQuery.data?.pagination.total ?? 0,
              showSizeChanger: true
            }}
            locale={{ emptyText: '暂无设备' }}
            onChange={(pagination) => {
              setDevicePage(pagination.current ?? 1);
              setDevicePageSize(pagination.pageSize ?? 20);
            }}
          />
        )}
      </Drawer>

      <Modal
        title={editingUser ? `编辑用户：${editingUser.username}` : '编辑用户'}
        open={editingUser !== null}
        confirmLoading={updateUserMutation.isPending}
        onCancel={() => {
          if (!updateUserMutation.isPending) {
            setEditingUser(null);
          }
        }}
        onOk={() => {
          void editForm.validateFields().then((values) => {
            if (editingUser) {
              updateUserMutation.mutate({ userId: editingUser.id, values });
            }
          });
        }}
      >
        <Form<UpdateUserRequest> form={editForm} layout="vertical">
          <Form.Item
            label="用户名"
            name="username"
            rules={[{ required: true, message: '请输入用户名' }, { min: 3, message: '用户名至少 3 个字符' }]}
          >
            <Input />
          </Form.Item>
          <Form.Item label="角色" name="role" rules={[{ required: true, message: '请选择角色' }]}>
            <Select options={[{ label: '普通用户', value: 'user' }, { label: '管理员', value: 'admin' }]} />
          </Form.Item>
          <Form.Item label="状态" name="status" rules={[{ required: true, message: '请选择状态' }]}>
            <Select options={[{ label: '启用', value: 'active' }, { label: '禁用', value: 'disabled' }]} />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        title={resettingUser ? `重置密码：${resettingUser.username}` : '重置密码'}
        open={resettingUser !== null}
        confirmLoading={resetPasswordMutation.isPending}
        onCancel={() => {
          if (!resetPasswordMutation.isPending) {
            setResettingUser(null);
            resetForm.resetFields();
          }
        }}
        onOk={() => {
          void resetForm.validateFields().then((values) => {
            if (resettingUser) {
              resetPasswordMutation.mutate({ userId: resettingUser.id, values });
            }
          });
        }}
      >
        <Form<ResetUserPasswordRequest> form={resetForm} layout="vertical">
          <Form.Item
            label="新密码"
            name="password"
            rules={[{ required: true, message: '请输入新密码' }, { min: 8, message: '密码至少 8 个字符' }]}
          >
            <Input.Password placeholder="至少 8 个字符" />
          </Form.Item>
        </Form>
      </Modal>
    </Space>
  );
}
