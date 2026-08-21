import { LockOutlined, ReloadOutlined, SafetyCertificateOutlined } from '@ant-design/icons';
import { Alert, Button, Card, Form, Input, Result, Space, Spin, Typography } from 'antd';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { buildDeviceRegistration, getOrCreateDeviceId } from '../deviceIdentity';
import {
  ControlPlaneSession,
  getControlPlaneErrorMessage,
  type ControlPlaneSessionSnapshot,
} from '../controlPlaneSession';
import { ControlPlaneHeartbeat } from './control-plane-heartbeat';

type LoginFormValues = {
  username: string;
  password: string;
};

type ActivationFormValues = {
  activationCode: string;
};

export function ControlPlaneGate({ children }: { children: ReactNode }) {
  const deviceId = useMemo(() => getOrCreateDeviceId(), []);
  const [session] = useState(() => new ControlPlaneSession());
  const [snapshot, setSnapshot] = useState<ControlPlaneSessionSnapshot>(() => session.getSnapshot());

  useEffect(() => {
    const unsubscribe = session.subscribe(setSnapshot);
    void session.restore(deviceId);
    return () => {
      unsubscribe();
      session.dispose();
    };
  }, [deviceId, session]);

  if (snapshot.status === 'loading') {
    return (
      <main className="desktop-auth-gate" aria-live="polite">
        <Spin size="large" tip="正在校验桌面端会话…" />
      </main>
    );
  }

  if (snapshot.status === 'unauthenticated') {
    return <LoginPanel snapshot={snapshot} onSubmit={(values) => void session.login(values.username, values.password, deviceId)} />;
  }

  if (snapshot.status === 'activation_required') {
    return (
      <ActivationPanel
        snapshot={snapshot}
        deviceId={deviceId}
        onSubmit={(activationCode) => void session.activate(activationCode, buildDeviceRegistration(deviceId))}
        onLogout={() => void session.logout()}
      />
    );
  }

  if (snapshot.status === 'error') {
    return (
      <main className="desktop-auth-gate">
        {snapshot.warning ? <Alert type="warning" showIcon message={snapshot.warning} /> : null}
        <Result
          status="error"
          title="控制面暂时不可用"
          subTitle={getControlPlaneErrorMessage(snapshot.error, '无法完成桌面端启动校验，请检查服务端地址和网络连接。')}
          extra={<Button type="primary" icon={<ReloadOutlined />} onClick={() => void session.restore(deviceId)}>重新连接</Button>}
        />
      </main>
    );
  }

  return (
    <>
      <ControlPlaneHeartbeat session={session} snapshot={snapshot} />
      {snapshot.warning ? <Alert className="desktop-auth-session-warning" type="warning" showIcon message={snapshot.warning} /> : null}
      <div className="desktop-auth-session-controls">
        <Typography.Text type="secondary">{snapshot.user?.username ?? '已登录'}</Typography.Text>
        <Button size="small" onClick={() => void session.logout()}>退出登录</Button>
      </div>
      {children}
    </>
  );
}

function LoginPanel({
  snapshot,
  onSubmit,
}: {
  snapshot: ControlPlaneSessionSnapshot;
  onSubmit: (values: LoginFormValues) => void;
}) {
  return (
    <main className="desktop-auth-gate">
      <Card className="desktop-auth-card" title="登录 autoLive" bordered={false}>
        <Space direction="vertical" size="middle" style={{ width: '100%' }}>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
            登录后还需要绑定当前设备。服务端加密主密钥不会进入桌面端。
          </Typography.Paragraph>
          {snapshot.warning ? <Alert type="warning" showIcon message={snapshot.warning} /> : null}
          {snapshot.error ? <Alert type="error" showIcon message={getControlPlaneErrorMessage(snapshot.error, '登录失败')} /> : null}
          <Form<LoginFormValues> layout="vertical" onFinish={onSubmit} requiredMark={false}>
            <Form.Item label="账号" name="username" rules={[{ required: true, message: '请输入账号' }]}>
              <Input autoComplete="username" prefix={<LockOutlined />} placeholder="请输入账号" />
            </Form.Item>
            <Form.Item label="密码" name="password" rules={[{ required: true, message: '请输入密码' }]}>
              <Input.Password autoComplete="current-password" placeholder="请输入密码" />
            </Form.Item>
            <Button type="primary" htmlType="submit" block>登录并继续</Button>
          </Form>
        </Space>
      </Card>
    </main>
  );
}

function ActivationPanel({
  snapshot,
  deviceId,
  onSubmit,
  onLogout,
}: {
  snapshot: ControlPlaneSessionSnapshot;
  deviceId: string;
  onSubmit: (activationCode: string) => void;
  onLogout: () => void;
}) {
  return (
    <main className="desktop-auth-gate">
      <Card className="desktop-auth-card" title="激活当前设备" bordered={false}>
        <Space direction="vertical" size="middle" style={{ width: '100%' }}>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
            当前账号已登录，请输入一次性激活码绑定此设备后使用本地播放功能。
          </Typography.Paragraph>
          <Alert
            type="info"
            showIcon
            icon={<SafetyCertificateOutlined />}
            message="设备标识已生成"
            description={<Typography.Text code copyable>{deviceId}</Typography.Text>}
          />
          {snapshot.warning ? <Alert type="warning" showIcon message={snapshot.warning} /> : null}
          {snapshot.error ? <Alert type="error" showIcon message={getControlPlaneErrorMessage(snapshot.error, '设备激活失败')} /> : null}
          <Form<ActivationFormValues> layout="vertical" onFinish={(values) => onSubmit(values.activationCode.trim())} requiredMark={false}>
            <Form.Item label="激活码" name="activationCode" rules={[{ required: true, message: '请输入激活码' }]}>
              <Input autoComplete="one-time-code" placeholder="请输入一次性激活码" />
            </Form.Item>
            <Space style={{ width: '100%' }}>
              <Button type="primary" htmlType="submit">绑定设备</Button>
              <Button onClick={onLogout}>退出登录</Button>
            </Space>
          </Form>
        </Space>
      </Card>
    </main>
  );
}
