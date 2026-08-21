import { LockOutlined, SafetyCertificateOutlined } from '@ant-design/icons';
import { Alert, App, Button, Card, Form, Input, Space, Typography } from 'antd';
import { useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { apiClient, ApiClientError, createRequestId } from '../../api/client';
import { clearSession } from '../auth/session';
import type { ChangeLocalAdminPasswordRequest, UserEnvelope } from '../../types/api';

export function AdminSecurityPage() {
  const { message } = App.useApp();
  const navigate = useNavigate();
  const [form] = Form.useForm<ChangeLocalAdminPasswordRequest & { confirmPassword: string }>();
  const [pending, setPending] = useState(false);
  const idempotencyKey = useRef<string | undefined>(undefined);

  const submit = async (values: ChangeLocalAdminPasswordRequest) => {
    setPending(true);
    const key = idempotencyKey.current ?? createRequestId();
    idempotencyKey.current = key;
    try {
      const response = await apiClient.post<UserEnvelope>('/api/v1/admin/auth/change-password', {
        body: values,
        headers: { 'Idempotency-Key': key }
      });
      idempotencyKey.current = undefined;
      void message.success(`管理员密码已更新（request_id：${response.request_id}），请重新登录`);
      clearSession();
      navigate('/login', { replace: true, state: { from: '/security' } });
    } catch (error) {
      if (!(error instanceof ApiClientError) || error.status !== 503) {
        idempotencyKey.current = undefined;
      }
      void message.error(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '管理员密码修改失败'
      );
    } finally {
      setPending(false);
    }
  };

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <div>
        <Typography.Title level={2} style={{ marginBottom: 8 }}>
          管理员安全
        </Typography.Title>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
          修改本地管理员密码后，当前管理员的所有会话会立即失效；请使用新密码重新登录。
        </Typography.Paragraph>
      </div>

      <Card
        title="轮换本地管理员密码"
        extra={<SafetyCertificateOutlined aria-label="管理员安全" />}
        style={{ maxWidth: 640 }}
      >
        <Alert
          showIcon
          type="warning"
          message="这是本地管理员专用操作"
          description="密码不会回显或写入日志。若会话存储暂时不可用，请使用相同页面提交重试。"
          style={{ marginBottom: 24 }}
        />
        <Form
          form={form}
          layout="vertical"
          onValuesChange={() => {
            idempotencyKey.current = undefined;
          }}
          onFinish={(values) => submit({ password: values.password })}
          autoComplete="off"
        >
          <Form.Item
            label="新密码"
            name="password"
            rules={[
              { required: true, message: '请输入新密码' },
              { min: 12, max: 256, message: '密码长度必须为 12 到 256 个字符' }
            ]}
          >
            <Input.Password prefix={<LockOutlined />} autoComplete="new-password" />
          </Form.Item>
          <Form.Item
            label="确认新密码"
            name="confirmPassword"
            dependencies={['password']}
            rules={[
              { required: true, message: '请再次输入新密码' },
              ({ getFieldValue }) => ({
                validator(_, value) {
                  if (!value || getFieldValue('password') === value) {
                    return Promise.resolve();
                  }
                  return Promise.reject(new Error('两次输入的密码不一致'));
                }
              })
            ]}
          >
            <Input.Password prefix={<LockOutlined />} autoComplete="new-password" />
          </Form.Item>
          <Button type="primary" htmlType="submit" loading={pending}>
            修改密码并退出
          </Button>
        </Form>
      </Card>
    </Space>
  );
}
