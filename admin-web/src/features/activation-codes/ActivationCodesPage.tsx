import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import dayjs, { type Dayjs } from 'dayjs';
import {
  Alert,
  App,
  Button,
  Card,
  DatePicker,
  Empty,
  Form,
  InputNumber,
  Modal,
  Result,
  Select,
  Space,
  Table,
  Typography
} from 'antd';
import { useMemo, useState } from 'react';
import { apiClient, ApiClientError, createRequestId } from '../../api/client';
import { StatusTag } from '../../components/StatusTag';
import { useAdminAuthorization } from '../admin-rbac/useAdminAuthorization';
import {
  ACTIVATION_CODE_EXPIRY_PRESETS,
  calculateActivationCodeExpiry
} from './activationCodeExpiry';
import type {
  ActivationCode,
  ActivationCodeEnvelope,
  ActivationCodeListResponse,
  CreateActivationCodeRequest,
  UserListResponse
} from '../../types/api';

type CreateActivationCodeFormValues = {
  user_id: string;
  expires_at: Dayjs;
  max_devices: number;
};

export function ActivationCodesPage() {
  const authorization = useAdminAuthorization();
  const canManageActivationCodes = authorization.can('activation_codes.manage');
  const canReadUsers = authorization.can('users.read');
  const canCreateActivationCodes = canManageActivationCodes && canReadUsers;
  const { message, modal } = App.useApp();
  const queryClient = useQueryClient();
  const [form] = Form.useForm<CreateActivationCodeFormValues>();
  const [createOpen, setCreateOpen] = useState(false);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [revokingCodeId, setRevokingCodeId] = useState<string | null>(null);
  const expiryPresets = useMemo(
    () =>
      ACTIVATION_CODE_EXPIRY_PRESETS.map((preset) => ({
        label: preset.label,
        value: () => dayjs(calculateActivationCodeExpiry(new Date(), preset.amount, preset.unit))
      })),
    []
  );

  const activationCodesQuery = useQuery({
    queryKey: ['activation-codes', page, pageSize],
    queryFn: () =>
      apiClient.get<ActivationCodeListResponse>('/api/v1/admin/activation-codes', {
        query: { page, page_size: pageSize }
      })
  });

  const activationCodeUsersQuery = useQuery({
    queryKey: ['admin-users', 'activation-code-picker', authorization.product],
    queryFn: () => {
      if (authorization.product === null) {
        throw new Error('当前产品范围尚未就绪');
      }

      return apiClient.get<UserListResponse>('/api/v1/admin/users', {
        query: { page: 1, page_size: 200, product: authorization.product }
      });
    },
    enabled: createOpen && canReadUsers && authorization.product !== null
  });

  const activationCodeUserOptions = useMemo(
    () =>
      (activationCodeUsersQuery.data?.items ?? [])
        .filter((user) => user.status === 'active')
        .map((user) => ({
          label: `${user.username}（${user.id}）`,
          value: user.id
        })),
    [activationCodeUsersQuery.data?.items]
  );

  const createActivationCodeMutation = useMutation({
    mutationFn: (values: CreateActivationCodeRequest) =>
      apiClient.post<ActivationCodeEnvelope>('/api/v1/admin/activation-codes', {
        body: values,
        headers: {
          'Idempotency-Key': createRequestId()
        }
      }),
    onSuccess: async (response) => {
      setCreateOpen(false);
      form.resetFields();
      await queryClient.invalidateQueries({ queryKey: ['activation-codes'] });
      modal.success({
        title: '激活码创建成功',
        content: (
          <Space direction="vertical">
            <Typography.Text>request_id：{response.request_id}</Typography.Text>
            <Typography.Text copyable strong>
              {response.activation_code.plain_code ?? '后端未返回一次性明文激活码'}
            </Typography.Text>
          </Space>
        )
      });
    },
    onError: (error) => {
      if (error instanceof ApiClientError) {
        void message.error(
          `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
        );
        return;
      }

      void message.error('创建激活码失败');
    }
  });

  const revokeActivationCodeMutation = useMutation({
    mutationFn: (codeId: string) =>
      apiClient.post<ActivationCodeEnvelope>(
        `/api/v1/admin/activation-codes/${codeId}/revoke`,
        {
          headers: {
            'Idempotency-Key': createRequestId()
          }
        }
      ),
    onSuccess: async (response) => {
      void message.success(`激活码已作废（request_id：${response.request_id}）`);
      await queryClient.invalidateQueries({ queryKey: ['activation-codes'] });
    },
    onError: (error) => {
      const errorMessage =
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '激活码作废失败';
      void message.error(errorMessage);
    },
    onSettled: () => {
      setRevokingCodeId(null);
    }
  });

  const columns = useMemo(
    () => [
      { title: 'ID', dataIndex: 'id', key: 'id' },
      {
        title: '状态',
        dataIndex: 'status',
        key: 'status',
        render: (status: ActivationCode['status']) => <StatusTag status={status} />
      },
      {
        title: '过期时间',
        dataIndex: 'expires_at',
        key: 'expires_at',
        render: (value: string) => new Date(value).toLocaleString('zh-CN')
      },
      {
        title: '已绑定设备',
        key: 'bound_devices',
        render: (_value: unknown, record: ActivationCode) =>
          `${record.bound_devices ?? 0} / ${record.max_devices}`
      },
      {
        title: '绑定账号',
        dataIndex: 'user_id',
        key: 'user_id',
        render: (value?: string) => value || '历史未分配（已作废）'
      },
      { title: '脱敏前缀', dataIndex: 'code_prefix', key: 'code_prefix', render: (value?: string) => value || '未生成' },
      { title: '核销设备', dataIndex: 'used_by_device_id', key: 'used_by_device_id', render: (value?: string) => value || '未核销' },
      {
        title: '核销时间',
        dataIndex: 'used_at',
        key: 'used_at',
        render: (value?: string) => (value ? new Date(value).toLocaleString('zh-CN') : '未核销')
      },
      {
        title: '操作',
        key: 'actions',
        render: (_value: unknown, record: ActivationCode) => (
          <Button
            danger
            type="link"
            disabled={
              !canManageActivationCodes ||
              !['active', 'used'].includes(record.status) ||
              revokeActivationCodeMutation.isPending
            }
            loading={revokingCodeId === record.id}
            onClick={() => {
              modal.confirm({
                title: '确认作废激活码？',
                content: '作废后该账号授权不能再绑定新设备，已绑定设备不受影响。',
                okText: '确认作废',
                cancelText: '取消',
                okButtonProps: { danger: true },
                onOk: () => {
                  setRevokingCodeId(record.id);
                  return revokeActivationCodeMutation.mutateAsync(record.id);
                }
              });
            }}
          >
            作废
          </Button>
        )
      }
    ],
    [canManageActivationCodes, modal, revokeActivationCodeMutation.isPending, revokingCodeId]
  );

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Space align="start" style={{ width: '100%', justifyContent: 'space-between' }}>
        <div>
          <Typography.Title level={2} style={{ margin: 0 }}>
            激活码管理
          </Typography.Title>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
            激活码创建时绑定账号并设置最多登录设备数；作废后不能再绑定新设备。
          </Typography.Paragraph>
        </div>

        <Button
          type="primary"
          disabled={!canCreateActivationCodes}
          title={canManageActivationCodes && !canReadUsers ? '创建激活码还需要 users.read 权限' : undefined}
          onClick={() => setCreateOpen(true)}
        >
          创建激活码
        </Button>
      </Space>

      {canManageActivationCodes && !canReadUsers ? (
        <Alert
          type="warning"
          showIcon
          message="缺少账号读取权限"
          description="创建激活码需要从当前产品的启用账号中选择绑定对象，请联系管理员补充 users.read 权限。"
        />
      ) : null}

      {activationCodesQuery.isError && activationCodesQuery.error instanceof ApiClientError && activationCodesQuery.error.status === 403 ? (
        <Result
          status="403"
          title="无权读取激活码列表"
          subTitle="服务端拒绝了当前会话的激活码读取请求，请刷新权限后重试。"
          extra={
            <Button
              type="primary"
              onClick={() => {
                void authorization.refresh();
                void activationCodesQuery.refetch();
              }}
            >
              重新验证权限
            </Button>
          }
        />
      ) : null}

      {activationCodesQuery.isError && (!(activationCodesQuery.error instanceof ApiClientError) || activationCodesQuery.error.status !== 403) ? (
        <Alert
          type="error"
          showIcon
          message="激活码列表加载失败"
          description={
            <Space direction="vertical" size="small">
              <Typography.Text>
                {activationCodesQuery.error instanceof ApiClientError
                  ? `${activationCodesQuery.error.message}${activationCodesQuery.error.requestId ? `（request_id：${activationCodesQuery.error.requestId}）` : ''}`
                  : '发生未知错误'}
              </Typography.Text>
              <Button onClick={() => void activationCodesQuery.refetch()}>重试</Button>
            </Space>
          }
        />
      ) : null}

      <Card>
        <Table<ActivationCode>
          rowKey="id"
          columns={columns}
          dataSource={activationCodesQuery.data?.items ?? []}
          loading={activationCodesQuery.isLoading}
          pagination={{
            current: page,
            pageSize,
            total: activationCodesQuery.data?.pagination.total ?? 0,
            showSizeChanger: true
          }}
          locale={{
            emptyText:
              activationCodesQuery.isLoading ? '加载中...' : <Empty description="暂无激活码数据" />
          }}
          onChange={(pagination) => {
            setPage(pagination.current ?? 1);
            setPageSize(pagination.pageSize ?? 20);
          }}
        />
        <Typography.Text type="secondary">
          最近请求 ID：{activationCodesQuery.data?.request_id ?? '暂无'}
        </Typography.Text>
      </Card>

      <Modal
        title="创建激活码"
        open={createOpen}
        confirmLoading={createActivationCodeMutation.isPending}
        okButtonProps={{
          disabled:
            !canCreateActivationCodes ||
            activationCodeUsersQuery.isLoading ||
            activationCodeUsersQuery.isError ||
            activationCodeUserOptions.length === 0
        }}
        onCancel={() => {
          if (!createActivationCodeMutation.isPending) {
            setCreateOpen(false);
          }
        }}
        onOk={() => {
          void form.validateFields().then((values) =>
            createActivationCodeMutation.mutate({
              user_id: values.user_id,
              expires_at: values.expires_at.toISOString(),
              max_devices: values.max_devices
            })
          );
        }}
      >
        {activationCodeUsersQuery.isError ? (
          <Alert
            type="error"
            showIcon
            message="绑定账号加载失败"
            description={
              activationCodeUsersQuery.error instanceof ApiClientError
                ? `${activationCodeUsersQuery.error.message}${activationCodeUsersQuery.error.requestId ? `（request_id：${activationCodeUsersQuery.error.requestId}）` : ''}`
                : '当前产品的账号列表暂时无法加载'
            }
            action={<Button onClick={() => void activationCodeUsersQuery.refetch()}>重试</Button>}
            style={{ marginBottom: 16 }}
          />
        ) : null}
        <Form<CreateActivationCodeFormValues> form={form} layout="vertical">
          <Form.Item
            label="绑定账号"
            name="user_id"
            rules={[{ required: true, message: '请选择绑定账号' }]}
          >
            <Select
              showSearch
              optionFilterProp="label"
              loading={activationCodeUsersQuery.isLoading}
              disabled={!canReadUsers || activationCodeUsersQuery.isError}
              options={activationCodeUserOptions}
              placeholder="请选择当前产品的启用账号"
              notFoundContent={
                activationCodeUsersQuery.isLoading ? '账号加载中...' : '暂无可绑定的启用账号'
              }
            />
          </Form.Item>
          <Form.Item
            label="过期时间"
            name="expires_at"
            rules={[{ required: true, message: '请输入过期时间' }]}
          >
            <DatePicker
              showTime
              format="YYYY-MM-DD HH:mm:ss"
              presets={expiryPresets}
              placeholder="请选择过期时间"
              style={{ width: '100%' }}
            />
          </Form.Item>
          <Form.Item
            label="最多登录设备数"
            name="max_devices"
            initialValue={1}
            rules={[{ required: true, message: '请输入最多登录设备数' }]}
          >
            <InputNumber min={1} max={100} precision={0} style={{ width: '100%' }} />
          </Form.Item>
        </Form>
      </Modal>
    </Space>
  );
}
