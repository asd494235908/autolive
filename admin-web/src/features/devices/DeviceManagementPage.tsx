import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Alert,
  App,
  Button,
  Card,
  Descriptions,
  Drawer,
  Empty,
  Result,
  Space,
  Table,
  Typography
} from 'antd';
import { useMemo, useState } from 'react';
import { apiClient, ApiClientError, createRequestId } from '../../api/client';
import { StatusTag } from '../../components/StatusTag';
import { useAdminAuthorization } from '../admin-rbac/useAdminAuthorization';
import type { DeviceEnvelope, DeviceListResponse, DeviceSummary, UnbindDeviceResponse } from '../../types/api';

function formatDiskSize(bytes?: number) {
  if (bytes === undefined) {
    return '未上报';
  }

  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function formatMemory(bytes?: number) {
  if (bytes === undefined) {
    return '未上报';
  }

  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export function DeviceManagementPage() {
  const authorization = useAdminAuthorization();
  const canManageDevices = authorization.can('devices.manage');
  const { message, modal } = App.useApp();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [unbindingDeviceId, setUnbindingDeviceId] = useState<string | null>(null);
  const [detailDeviceId, setDetailDeviceId] = useState<string | null>(null);

  const devicesQuery = useQuery({
    queryKey: ['admin-devices', page, pageSize],
    queryFn: () =>
      apiClient.get<DeviceListResponse>('/api/v1/admin/devices', {
        query: { page, page_size: pageSize }
      })
  });

  const deviceDetailQuery = useQuery({
    queryKey: ['admin-device', detailDeviceId],
    queryFn: () => apiClient.get<DeviceEnvelope>(`/api/v1/admin/devices/${detailDeviceId}`),
    enabled: detailDeviceId !== null
  });

  const disableDeviceMutation = useMutation({
    mutationFn: (deviceId: string) =>
      apiClient.post<DeviceEnvelope>(`/api/v1/admin/devices/${deviceId}/disable`, {
        headers: { 'Idempotency-Key': createRequestId() }
      }),
    onSuccess: async (response) => {
      void message.success(`设备已禁用：${response.device.device_name}`);
      await queryClient.invalidateQueries({ queryKey: ['admin-devices'] });
    },
    onError: (error) => {
      void message.error(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '禁用设备失败'
      );
    }
  });

  const unbindDeviceMutation = useMutation({
    mutationFn: (deviceId: string) =>
      apiClient.post<UnbindDeviceResponse>(`/api/v1/admin/devices/${deviceId}/unbind`, {
        headers: { 'Idempotency-Key': createRequestId() }
      }),
    onSuccess: async (response) => {
      void message.success(`设备已解除绑定：${response.device_name}`);
      await queryClient.invalidateQueries({ queryKey: ['admin-devices'] });
    },
    onError: (error) => {
      void message.error(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '解除设备绑定失败'
      );
    },
    onSettled: () => {
      setUnbindingDeviceId(null);
    }
  });

  const columns = useMemo(
    () => [
      { title: '设备名', dataIndex: 'device_name', key: 'device_name' },
      { title: '用户 ID', dataIndex: 'user_id', key: 'user_id' },
      { title: '平台', dataIndex: 'platform', key: 'platform' },
      { title: '客户端版本', dataIndex: 'app_version', key: 'app_version' },
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
      {
        title: '磁盘剩余',
        dataIndex: 'disk_free_bytes',
        key: 'disk_free_bytes',
        render: (value?: number) => formatDiskSize(value)
      },
      {
        title: '运行时信息',
        key: 'runtime_metrics',
        render: (_: unknown, record: DeviceSummary) => {
          const os = [record.runtime_os_name, record.runtime_os_version]
            .filter(Boolean)
            .join(' ');
          const memory =
            record.memory_total_bytes === undefined
              ? '内存未上报'
              : `内存 ${formatMemory(record.memory_available_bytes)} / ${formatMemory(record.memory_total_bytes)}`;
          const cpu = record.cpu_logical_cores
            ? `CPU ${record.cpu_logical_cores} 线程`
            : 'CPU 未上报';
          return `${os || '系统未上报'}；${memory}；${cpu}`;
        }
      },
      {
        title: '当前播放',
        key: 'playback',
        render: (_: unknown, record: DeviceSummary) =>
          record.current_media_name || record.playback_state
            ? `${record.current_media_name || '未命名媒体'}（${record.playback_state || '未上报'}）`
            : '未上报'
      },
      {
        title: '最后心跳',
        dataIndex: 'last_seen_at',
        key: 'last_seen_at',
        render: (value: string) => new Date(value).toLocaleString('zh-CN')
      },
      {
        title: '操作',
        key: 'actions',
        render: (_: unknown, record: DeviceSummary) => (
          <Space>
            <Button onClick={() => setDetailDeviceId(record.id)}>详情</Button>
            <Button
              danger
              disabled={!canManageDevices || record.status === 'disabled' || record.status === 'revoked'}
              loading={disableDeviceMutation.isPending}
              onClick={() => {
                modal.confirm({
                  title: '确认禁用设备',
                  content: `将禁用设备“${record.device_name}”，客户端下次鉴权时会停止受保护能力。`,
                  okText: '确认禁用',
                  cancelText: '返回',
                  onOk: () => disableDeviceMutation.mutateAsync(record.id)
                });
              }}
            >
              禁用
            </Button>
            <Button
              disabled={!canManageDevices || record.status === 'pending_activation' || unbindDeviceMutation.isPending}
              loading={unbindingDeviceId === record.id}
              onClick={() => {
                modal.confirm({
                  title: '确认解除设备绑定',
                  content: `解除“${record.device_name}”后，原用户将失去该设备的受保护访问；重新使用必须输入新的激活码。`,
                  okText: '确认解绑',
                  cancelText: '返回',
                  onOk: () => {
                    setUnbindingDeviceId(record.id);
                    return unbindDeviceMutation.mutateAsync(record.id);
                  }
                });
              }}
            >
              解绑
            </Button>
          </Space>
        )
      }
    ],
    [disableDeviceMutation, modal, setDetailDeviceId, unbindDeviceMutation, unbindingDeviceId]
  );

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <div>
        <Typography.Title level={2} style={{ margin: 0 }}>
          设备管理
        </Typography.Title>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
          对齐 `/api/v1/admin/devices` 列表、禁用和解绑接口；解绑后设备进入待激活状态。
        </Typography.Paragraph>
      </div>

      {devicesQuery.isError && devicesQuery.error instanceof ApiClientError && devicesQuery.error.status === 403 ? (
        <Result
          status="403"
          title="无权读取设备列表"
          subTitle="服务端拒绝了当前会话的设备读取请求，请刷新权限后重试。"
          extra={
            <Button
              type="primary"
              onClick={() => {
                void authorization.refresh();
                void devicesQuery.refetch();
              }}
            >
              重新验证权限
            </Button>
          }
        />
      ) : null}

      {devicesQuery.isError && (!(devicesQuery.error instanceof ApiClientError) || devicesQuery.error.status !== 403) ? (
        <Alert
          type="error"
          showIcon
          message="设备列表加载失败"
          description={
            <Space direction="vertical" size="small">
              <Typography.Text>
                {devicesQuery.error instanceof ApiClientError
                  ? `${devicesQuery.error.message}${devicesQuery.error.requestId ? `（request_id：${devicesQuery.error.requestId}）` : ''}`
                  : '发生未知错误'}
              </Typography.Text>
              <Button onClick={() => void devicesQuery.refetch()}>重试</Button>
            </Space>
          }
        />
      ) : null}

      <Card>
        <Table<DeviceSummary>
          rowKey="id"
          columns={columns}
          dataSource={devicesQuery.data?.items ?? []}
          loading={devicesQuery.isLoading}
          pagination={{
            current: page,
            pageSize,
            total: devicesQuery.data?.pagination.total ?? 0,
            showSizeChanger: true
          }}
          locale={{
            emptyText: devicesQuery.isLoading ? '加载中...' : <Empty description="暂无设备数据" />
          }}
          onChange={(pagination) => {
            setPage(pagination.current ?? 1);
            setPageSize(pagination.pageSize ?? 20);
          }}
        />
        <Typography.Text type="secondary">
          最近请求 ID：{devicesQuery.data?.request_id ?? '暂无'}
        </Typography.Text>
      </Card>
      <Drawer
        title="设备详情"
        open={detailDeviceId !== null}
        width={480}
        onClose={() => setDetailDeviceId(null)}
      >
        {deviceDetailQuery.isError && deviceDetailQuery.error instanceof ApiClientError && deviceDetailQuery.error.status === 403 ? (
          <Result status="403" title="无权读取设备详情" subTitle="当前会话缺少 devices.read 权限。" />
        ) : null}
        {deviceDetailQuery.isError && (!(deviceDetailQuery.error instanceof ApiClientError) || deviceDetailQuery.error.status !== 403) ? (
          <Alert
            type="error"
            showIcon
            message="设备详情加载失败"
            description={
              deviceDetailQuery.error instanceof ApiClientError
                ? `${deviceDetailQuery.error.message}${deviceDetailQuery.error.requestId ? `（request_id：${deviceDetailQuery.error.requestId}）` : ''}`
                : '发生未知错误'
            }
          />
        ) : deviceDetailQuery.isLoading ? (
          <Typography.Text type="secondary">加载中...</Typography.Text>
        ) : deviceDetailQuery.data?.device ? (
          <Descriptions column={1} bordered size="small">
            <Descriptions.Item label="设备 ID">{deviceDetailQuery.data.device.id}</Descriptions.Item>
            <Descriptions.Item label="设备名">{deviceDetailQuery.data.device.device_name}</Descriptions.Item>
            <Descriptions.Item label="用户 ID">{deviceDetailQuery.data.device.user_id || '未绑定'}</Descriptions.Item>
            <Descriptions.Item label="状态">{deviceDetailQuery.data.device.status}</Descriptions.Item>
            <Descriptions.Item label="在线状态">
              {deviceDetailQuery.data.device.online ? '在线' : '离线'}
            </Descriptions.Item>
            <Descriptions.Item label="当前播放">
              {deviceDetailQuery.data.device.current_media_name || '未上报'}
              {deviceDetailQuery.data.device.playback_state
                ? `（${deviceDetailQuery.data.device.playback_state}）`
                : ''}
            </Descriptions.Item>
            <Descriptions.Item label="最后心跳">
              {new Date(deviceDetailQuery.data.device.last_seen_at).toLocaleString('zh-CN')}
            </Descriptions.Item>
          </Descriptions>
        ) : (
          <Empty description="暂无设备详情" />
        )}
      </Drawer>
    </Space>
  );
}
