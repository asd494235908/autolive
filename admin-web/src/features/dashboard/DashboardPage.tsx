import { useQuery } from '@tanstack/react-query';
import { Alert, Card, Col, Descriptions, Row, Space, Statistic, Typography } from 'antd';
import { apiClient, ApiClientError } from '../../api/client';
import type { HealthResponse } from '../../types/api';

export function DashboardPage() {
  const healthQuery = useQuery({
    queryKey: ['health-status'],
    queryFn: async () => apiClient.get<HealthResponse>('/api/v1/health')
  });

  const healthData = healthQuery.data;
  const errorMessage =
    healthQuery.error instanceof ApiClientError
      ? `${healthQuery.error.message}${healthQuery.error.requestId ? `（request_id：${healthQuery.error.requestId}）` : ''}`
      : '健康检查请求失败';

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <div>
        <Typography.Title level={2} style={{ marginBottom: 8 }}>
          系统概览
        </Typography.Title>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
          当前为管理端控制面工作台，已接入健康检查、用户、设备、激活码和大模型号池管理。
        </Typography.Paragraph>
      </div>

      <Row gutter={[16, 16]}>
        <Col xs={24} lg={12}>
          <Card title="服务健康状态">
            {healthQuery.isLoading ? (
              <Statistic title="状态" value="检查中" />
            ) : healthQuery.isError ? (
              <Alert
                type="warning"
                showIcon
                message="功能建设中"
                description={`后端健康接口暂不可用：${errorMessage}`}
              />
            ) : (
              <Descriptions column={1} size="small">
                <Descriptions.Item label="状态">
                  {healthData?.status ?? '未返回'}
                </Descriptions.Item>
                <Descriptions.Item label="服务名">
                  {healthData?.service ?? '未返回'}
                </Descriptions.Item>
                <Descriptions.Item label="版本">
                  {healthData?.version ?? '未返回'}
                </Descriptions.Item>
                <Descriptions.Item label="request_id">
                  {healthData?.request_id ?? '未返回'}
                </Descriptions.Item>
              </Descriptions>
            )}
          </Card>
        </Col>

        <Col xs={24} lg={12}>
          <Card title="当前阶段说明">
            <Alert
              type="info"
              showIcon
              message="控制面能力持续补齐"
              description="号池摘要与新增账号已可用；完整账号生命周期、审计日志仍是后续阶段。本页面不处理视频文件，媒体生成与本地展示由 Rust 桌面端负责。"
            />
          </Card>
        </Col>
      </Row>
    </Space>
  );
}
