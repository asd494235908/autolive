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
  Space,
  Table,
  Typography
} from 'antd';
import { useMemo, useState } from 'react';
import { apiClient, ApiClientError, createRequestId } from '../../api/client';
import { StatusTag } from '../../components/StatusTag';
import type {
  ActivationCode,
  ActivationCodeEnvelope,
  ActivationCodeListResponse,
  CreateActivationCodeRequest
} from '../../types/api';

export function ActivationCodesPage() {
  const { message, modal } = App.useApp();
  const queryClient = useQueryClient();
  const [form] = Form.useForm<{ expires_at: string }>();
  const [createOpen, setCreateOpen] = useState(false);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [revokingCodeId, setRevokingCodeId] = useState<string | null>(null);

  const activationCodesQuery = useQuery({
    queryKey: ['activation-codes', page, pageSize],
    queryFn: () =>
      apiClient.get<ActivationCodeListResponse>('/api/v1/admin/activation-codes', {
        query: { page, page_size: pageSize }
      })
  });

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
      { title: '可绑定设备数', dataIndex: 'max_devices', key: 'max_devices' },
      {
        title: '操作',
        key: 'actions',
        render: (_value: unknown, record: ActivationCode) => (
          <Button
            danger
            type="link"
            disabled={record.status !== 'active' || revokeActivationCodeMutation.isPending}
            loading={revokingCodeId === record.id}
            onClick={() => {
              modal.confirm({
                title: '确认作废激活码？',
                content: '作废后该激活码不能再绑定设备，已绑定设备不受影响。',
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
    [modal, revokeActivationCodeMutation.isPending, revokingCodeId]
  );

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Space align="start" style={{ width: '100%', justifyContent: 'space-between' }}>
        <div>
          <Typography.Title level={2} style={{ margin: 0 }}>
            激活码管理
          </Typography.Title>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
            对齐 `/api/v1/admin/activation-codes` 的列表、创建和作废接口；作废只影响未核销激活码。
          </Typography.Paragraph>
        </div>

        <Button type="primary" onClick={() => setCreateOpen(true)}>
          创建激活码
        </Button>
      </Space>

      {activationCodesQuery.isError ? (
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
        onCancel={() => {
          if (!createActivationCodeMutation.isPending) {
            setCreateOpen(false);
          }
        }}
        onOk={() => {
          void form.validateFields().then((values) =>
            createActivationCodeMutation.mutate({
              expires_at: values.expires_at,
              max_devices: 1
            })
          );
        }}
      >
        <Alert
          type="info"
          showIcon
          style={{ marginBottom: 16 }}
          message="请输入 RFC3339 时间"
          description="示例：2026-08-12T20:00:00+08:00"
        />
        <Form<{ expires_at: string }> form={form} layout="vertical">
          <Form.Item
            label="过期时间"
            name="expires_at"
            rules={[{ required: true, message: '请输入过期时间' }]}
          >
            <Input placeholder="2026-08-12T20:00:00+08:00" />
          </Form.Item>
        </Form>
      </Modal>
    </Space>
  );
}
