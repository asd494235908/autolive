import { useMutation } from '@tanstack/react-query';
import {
  Alert,
  App as AntdApp,
  Button,
  Checkbox,
  Form,
  Input,
  type InputRef,
} from 'antd';
import { useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { apiClient, ApiClientError } from '../../api/client';
import type { LoginRequest, LoginResponse } from '../../types/api';
import yingshengLoginVisual from './assets/yingsheng-login.png';
import './LoginPage.css';
import { saveSession } from './session';

type LoginFormValues = Pick<LoginRequest, 'username' | 'password'>;

export function LoginPage() {
  const { message } = AntdApp.useApp();
  const [feedback, setFeedback] = useState<string | null>(null);
  const [passwordVisible, setPasswordVisible] = useState(false);
  const [rememberAccount, setRememberAccount] = useState(true);
  const [form] = Form.useForm<LoginFormValues>();
  const passwordInputRef = useRef<InputRef>(null);
  const navigate = useNavigate();
  const username = Form.useWatch('username', form) ?? '';
  const password = Form.useWatch('password', form) ?? '';

  const loginMutation = useMutation({
    mutationFn: async (values: LoginFormValues) =>
      apiClient.post<LoginResponse>('/api/v1/auth/login', {
        body: { ...values, product: 'autolive' },
      }),
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
    },
  });

  const showUnavailable = (content: string) => {
    void message.info(content);
  };

  return (
    <main className="login-page">
      <img
        className="login-page__backdrop"
        src={yingshengLoginVisual}
        alt=""
        aria-hidden="true"
        draggable={false}
      />

      <section className="login-stage" aria-labelledby="login-title">
        <img
          className="login-stage__visual"
          src={yingshengLoginVisual}
          alt=""
          aria-hidden="true"
          draggable={false}
        />
        <h1 id="login-title" className="login-visually-hidden">
          映声工坊管理端登录
        </h1>
        <p className="login-visually-hidden">
          使用管理员账号和密码登录映声工坊管理端。
        </p>

        {feedback ? (
          <Alert
            className="login-feedback"
            type="warning"
            showIcon
            closable
            message={feedback}
            onClose={() => setFeedback(null)}
          />
        ) : null}

        <Button
          className="login-hotspot login-hotspot--account-tab"
          type="text"
          aria-label="账号登录"
        >
          账号登录
        </Button>
        <Button
          className="login-hotspot login-hotspot--phone-tab"
          type="text"
          aria-label="手机登录暂未开放"
          onClick={() => showUnavailable('手机登录暂未开放，请使用管理员账号登录。')}
        >
          手机登录
        </Button>

        <Form<LoginFormValues>
          form={form}
          component="form"
          initialValues={{ username: '', password: '' }}
          onFinish={(values) => {
            setFeedback(null);
            loginMutation.mutate(values);
          }}
          onFinishFailed={() => setFeedback('请输入账号和密码。')}
        >
          <label className="login-visually-hidden" htmlFor="admin-username">
            账号
          </label>
          <div className="login-field login-field--username">
            <Form.Item
              name="username"
              noStyle
              rules={[{ required: true, message: '请输入账号' }]}
            >
              <Input
                id="admin-username"
                className="login-field__input"
                autoComplete="username"
                aria-label="请输入账号"
                placeholder="请输入账号"
                onChange={() => setFeedback(null)}
              />
            </Form.Item>
          </div>

          <label className="login-visually-hidden" htmlFor="admin-password">
            密码
          </label>
          <div className="login-field login-field--password">
            <Form.Item
              name="password"
              noStyle
              rules={[{ required: true, message: '请输入密码' }]}
            >
              <Input
                ref={passwordInputRef}
                id="admin-password"
                className="login-field__input"
                type={passwordVisible ? 'text' : 'password'}
                autoComplete="current-password"
                aria-label="请输入密码"
                placeholder="请输入密码"
                onChange={() => setFeedback(null)}
              />
            </Form.Item>
          </div>

          {username ? (
            <span className="login-mirror login-mirror--username" aria-hidden="true">
              {username}
            </span>
          ) : null}
          {password ? (
            <span className="login-mirror login-mirror--password" aria-hidden="true">
              {passwordVisible ? password : '•'.repeat(password.length)}
            </span>
          ) : null}

          <Button
            className="login-hotspot login-hotspot--eye"
            type="text"
            aria-label={passwordVisible ? '隐藏密码' : '显示密码'}
            onClick={() => {
              setPasswordVisible((visible) => !visible);
              passwordInputRef.current?.focus();
            }}
          >
            {passwordVisible ? '隐藏密码' : '显示密码'}
          </Button>

          <span className="login-check-control">
            <Checkbox
              checked={rememberAccount}
              aria-label="记住我"
              onChange={(event) => setRememberAccount(event.target.checked)}
            >
              记住我
            </Checkbox>
          </span>
          {!rememberAccount ? <span className="login-unchecked" aria-hidden="true" /> : null}

          <Button
            className="login-hotspot login-hotspot--forgot"
            type="text"
            aria-label="忘记密码暂未开放"
            onClick={() => showUnavailable('忘记密码暂未开放，请联系系统管理员。')}
          >
            忘记密码
          </Button>
          <Button
            className="login-hotspot login-hotspot--submit"
            type="primary"
            htmlType="submit"
            loading={loginMutation.isPending}
            aria-label="登录"
          >
            登录
          </Button>
          {loginMutation.isPending ? (
            <span className="login-submit-status" role="status">
              正在登录…
            </span>
          ) : null}
        </Form>

        <Button
          className="login-hotspot login-hotspot--register"
          type="text"
          aria-label="立即注册暂未开放"
          onClick={() => showUnavailable('管理端不开放注册，请联系系统管理员创建账号。')}
        >
          立即注册
        </Button>
      </section>
    </main>
  );
}
