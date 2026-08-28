import { LockOutlined } from '@ant-design/icons';
import { Alert, Button, Card, Checkbox, Form, Input, Space, Typography, type InputRef } from 'antd';
import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import {
  clearLegacyRememberedAuthCredentials,
  clearRememberedLogin,
  loadRememberedLogin,
} from '../authFormMemory';
import { getControlPlaneErrorMessage, type ControlPlaneSessionSnapshot } from '../controlPlaneSession';

export type AccessFormValues = {
  username: string;
  password: string;
  rememberLogin: boolean;
};

export type AccessSubmitResult = {
  status: ControlPlaneSessionSnapshot['status'];
  errorMessage?: string;
};

export type CredentialNotice = {
  type: 'success' | 'warning';
  message: string;
} | null;

type ControlPlaneAccessPanelProps = {
  snapshot: ControlPlaneSessionSnapshot;
  deviceId: string;
  busy: boolean;
  credentialNotice: CredentialNotice;
  onCredentialNotice: Dispatch<SetStateAction<CredentialNotice>>;
  onSubmit: (values: AccessFormValues) => Promise<AccessSubmitResult>;
};

export function ControlPlaneAccessPanel({
  snapshot,
  deviceId,
  busy,
  credentialNotice,
  onCredentialNotice,
  onSubmit,
}: ControlPlaneAccessPanelProps) {
  const [form] = Form.useForm<AccessFormValues>();
  const [memoryLoading, setMemoryLoading] = useState(true);
  const [memoryMutating, setMemoryMutating] = useState(false);
  const passwordInputRef = useRef<InputRef>(null);
  const controlsDisabled = memoryLoading || memoryMutating || busy;

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        await clearLegacyRememberedAuthCredentials(deviceId);
        const remembered = await loadRememberedLogin(deviceId);
        if (cancelled) return;
        form.setFieldsValue({
          username: remembered.username,
          password: '',
          rememberLogin: remembered.remember,
        });
      } catch (error) {
        if (cancelled) return;
        onCredentialNotice({
          type: 'warning',
          message: getControlPlaneErrorMessage(error, '读取已保存的账号失败，请手动输入。'),
        });
      } finally {
        if (!cancelled) setMemoryLoading(false);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [deviceId, form, onCredentialNotice]);

  const clearLoginMemory = async (clearForm: boolean) => {
    setMemoryMutating(true);
    onCredentialNotice(null);
    try {
      await clearRememberedLogin(deviceId);
      form.setFieldValue('rememberLogin', false);
      if (clearForm) form.setFieldsValue({ username: '', password: '' });
      onCredentialNotice({ type: 'success', message: '已清除当前设备记住的账号。' });
    } catch (error) {
      onCredentialNotice({
        type: 'warning',
        message: getControlPlaneErrorMessage(error, '清除已记住账号失败。'),
      });
    } finally {
      setMemoryMutating(false);
    }
  };

  const submitAccess = async (values: AccessFormValues) => {
    onCredentialNotice(null);
    form.setFields([{ name: 'password', errors: [] }]);
    const result = await onSubmit(values);
    if (result.status === 'unauthenticated') {
      form.setFields([{ name: 'password', errors: [result.errorMessage ?? '登录失败，请检查账号和密码'] }]);
      passwordInputRef.current?.focus();
    }
  };

  return (
    <div className="desktop-auth-page">
      <main className="desktop-auth-gate">
        <Card className="desktop-auth-card desktop-access-card" bordered={false}>
          <div className="desktop-login-header">
            <img className="desktop-login-logo" src="/app-icon.png" alt="GpAutoLive 产品 Logo" />
            <div>
              <Typography.Title className="desktop-login-title" level={3}>账号登录</Typography.Title>
              <Typography.Text type="secondary">设备授权由服务端按账号自动校验。</Typography.Text>
            </div>
          </div>

          <Space direction="vertical" size="middle" style={{ width: '100%' }}>
            <div className="desktop-auth-notices" aria-live="polite">
              {snapshot.warning ? <Alert type="warning" showIcon message={snapshot.warning} /> : null}
              {snapshot.error ? (
                <Alert type="error" showIcon message={getControlPlaneErrorMessage(snapshot.error, '登录或设备授权失败')} />
              ) : null}
              {credentialNotice ? <Alert type={credentialNotice.type} showIcon message={credentialNotice.message} /> : null}
              {memoryLoading ? <Alert type="info" showIcon message="正在读取已保存的信息…" /> : null}
            </div>

            <Form<AccessFormValues>
              form={form}
              layout="vertical"
              initialValues={{ username: '', password: '', rememberLogin: false }}
              onFinish={(values) => void submitAccess(values)}
              requiredMark={false}
            >
              <section className="desktop-access-section" aria-labelledby="desktop-account-section-title">
                <div className="desktop-access-section-heading">
                  <div>
                    <Typography.Title id="desktop-account-section-title" level={4}>账号密码</Typography.Title>
                    <Typography.Text type="secondary">使用与管理后台一致的账号和密码。</Typography.Text>
                  </div>
                </div>
                <Form.Item label="账号" name="username" rules={[
                  { required: true, message: '请输入账号' },
                  { min: 3, max: 64, message: '账号长度需要在 3 到 64 个字符之间' },
                ]}>
                  <Input disabled={controlsDisabled} autoComplete="username" prefix={<LockOutlined />} placeholder="请输入账号" maxLength={64} />
                </Form.Item>
                <Form.Item label="密码" name="password" rules={[
                  { required: true, message: '请输入密码' },
                  { min: 8, max: 256, message: '密码长度需要在 8 到 256 个字符之间' },
                  {
                    validator: async (_, value: unknown) => {
                      if (typeof value !== 'string' || new TextEncoder().encode(value).byteLength <= 256) return;
                      throw new Error('密码 UTF-8 编码后不能超过 256 字节');
                    },
                  },
                ]}>
                  <Input.Password ref={passwordInputRef} disabled={controlsDisabled} autoComplete="current-password" placeholder="请输入密码" maxLength={256} />
                </Form.Item>
                <div className="desktop-auth-memory-row">
                  <Form.Item name="rememberLogin" valuePropName="checked" noStyle>
                    <Checkbox
                      disabled={controlsDisabled}
                      onChange={(event) => {
                        if (!event.target.checked) void clearLoginMemory(false);
                      }}
                    >
                      记住账号
                    </Checkbox>
                  </Form.Item>
                  <Button type="link" size="small" disabled={controlsDisabled} onClick={() => void clearLoginMemory(true)}>
                    清除已记住账号
                  </Button>
                </div>
                <Typography.Paragraph className="desktop-auth-memory-help" type="secondary">
                  只在当前设备偏好中记住账号；密码不保存。保持登录使用系统钥匙串中的会话凭据。
                </Typography.Paragraph>
                <Typography.Text className="desktop-auth-device-id" type="secondary" copyable>
                  当前设备：{deviceId}
                </Typography.Text>
              </section>

              <Space className="desktop-auth-actions" direction="vertical" size="small">
                <Button type="primary" htmlType="submit" block loading={busy} disabled={controlsDisabled}>
                  登录并进入工作台
                </Button>
              </Space>
            </Form>
          </Space>
        </Card>
      </main>
    </div>
  );
}
