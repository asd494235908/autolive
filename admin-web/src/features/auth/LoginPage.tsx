import { useMutation } from '@tanstack/react-query';
import { Alert, Button, Card, Form, Input, Space, Typography } from 'antd';
import { useNavigate } from 'react-router-dom';
import { useState } from 'react';
import { apiClient, ApiClientError } from '../../api/client';
import type { LoginRequest, LoginResponse } from '../../types/api';
import { saveSession } from './session';

export function LoginPage() {
  const [feedback, setFeedback] = useState<string | null>(null);
  const [form] = Form.useForm<LoginRequest>();
  const navigate = useNavigate();

  const loginMutation = useMutation({
    mutationFn: async (values: LoginRequest) =>
      apiClient.post<LoginResponse>('/api/v1/auth/login', { body: values }),
    onSuccess: (response) => {
      saveSession({ tokens: response.tokens, user: response.user });
      navigate('/', { replace: true });
    },
    onError: (error) => {
      if (error instanceof ApiClientError) {
        setFeedback(
          `登录未完成：${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
        );
        return;
      }

      setFeedback('登录未完成：发生未知异常。');
    }
  });

  return (
    <div
      style={{
        minHeight: '100vh',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 24,
        background: '#f5f5f5'
      }}
    >
      <Card
        title="AutoLive 管理端登录"
        style={{ width: '100%', maxWidth: 420 }}
      >
        <Space direction="vertical" size="large" style={{ width: '100%' }}>
          <Alert
            type="info"
            showIcon
            message="管理端登录"
            description="登录成功后会在当前浏览器会话中保存短期访问凭证，并用于调用受保护的管理接口。"
          />

          {feedback ? <Alert type="warning" showIcon message={feedback} /> : null}

          <Form<LoginRequest>
            form={form}
            layout="vertical"
            initialValues={{ username: '', password: '' }}
            onFinish={(values) => {
              setFeedback(null);
              loginMutation.mutate(values);
            }}
          >
            <Form.Item
              label="用户名"
              name="username"
              rules={[{ required: true, message: '请输入用户名' }]}
            >
              <Input autoComplete="username" placeholder="请输入用户名" />
            </Form.Item>

            <Form.Item
              label="密码"
              name="password"
              rules={[{ required: true, message: '请输入密码' }]}
            >
              <Input.Password
                autoComplete="current-password"
                placeholder="请输入密码"
              />
            </Form.Item>

            <Button
              type="primary"
              htmlType="submit"
              block
              loading={loginMutation.isPending}
            >
              登录
            </Button>
          </Form>

          <Typography.Text type="secondary">
            如后端尚未启动，页面会明确显示连接失败；凭证只保存在当前浏览器会话。
          </Typography.Text>
        </Space>
      </Card>
    </div>
  );
}
