import { ReloadOutlined } from '@ant-design/icons';
import { Alert, Button, Result, Space, Spin, Typography } from 'antd';
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Navigate, Route, Routes } from 'react-router-dom';
import {
  clearRememberedLogin,
  saveRememberedLogin,
} from '../authFormMemory';
import { buildDeviceRegistration, getOrCreateDeviceId } from '../deviceIdentity';
import {
  ControlPlaneSession,
  getControlPlaneErrorMessage,
  type ControlPlaneSessionSnapshot,
} from '../controlPlaneSession';
import { isConfirmedDesktopAccess } from '../controlPlaneGatePolicy';
import {
  ControlPlaneAccessPanel,
  type AccessFormValues,
  type AccessSubmitResult,
  type CredentialNotice,
} from './control-plane-access-panel';
import { ControlPlaneHeartbeat } from './control-plane-heartbeat';

const CREDENTIAL_SUCCESS_NOTICE_DURATION_MS = 4_000;

function useCredentialNotice() {
  const [notice, setNotice] = useState<CredentialNotice>(null);

  useEffect(() => {
    if (notice?.type !== 'success') return;
    const timer = window.setTimeout(() => {
      setNotice((current) => current === notice ? null : current);
    }, CREDENTIAL_SUCCESS_NOTICE_DURATION_MS);
    return () => window.clearTimeout(timer);
  }, [notice]);

  return [notice, setNotice] as const;
}

export function ControlPlaneGate({ children }: { children: ReactNode }) {
  const deviceId = useMemo(() => getOrCreateDeviceId(), []);
  const [session] = useState(() => new ControlPlaneSession());
  const [snapshot, setSnapshot] = useState<ControlPlaneSessionSnapshot>(() => session.getSnapshot());
  const [credentialNotice, setCredentialNotice] = useCredentialNotice();
  const [accessBusy, setAccessBusy] = useState(false);
  const accessSubmitInFlightRef = useRef(false);

  useEffect(() => {
    const unsubscribe = session.subscribe(setSnapshot);
    void session.restore(buildDeviceRegistration(deviceId));
    return () => {
      unsubscribe();
      session.dispose();
    };
  }, [deviceId, session]);

  const handleAccess = async (values: AccessFormValues): Promise<AccessSubmitResult> => {
    if (accessSubmitInFlightRef.current) {
      return { status: session.getSnapshot().status };
    }
    accessSubmitInFlightRef.current = true;
    setAccessBusy(true);
    setCredentialNotice(null);

    const username = values.username.trim();
    const successMessages: string[] = [];
    const warningMessages: string[] = [];

    try {
      const result = await session.login(username, values.password, buildDeviceRegistration(deviceId));
      if (result.accessToken) {
        try {
          if (values.rememberLogin) {
            await saveRememberedLogin(deviceId, username, values.password);
            successMessages.push('账号和密码已保存到当前设备的安全凭据库。');
          } else {
            await clearRememberedLogin(deviceId);
          }
        } catch (error) {
          warningMessages.push(getControlPlaneErrorMessage(error, '账号和密码保存失败，本次登录仍然有效。'));
        }
      }

      publishCredentialNotice(successMessages, warningMessages, setCredentialNotice);
      return {
        status: result.status,
        errorMessage: result.error
          ? getControlPlaneErrorMessage(
              result.error,
              result.status === 'activation_required' ? '设备授权失败' : '登录失败',
            )
          : undefined,
      };
    } finally {
      accessSubmitInFlightRef.current = false;
      setAccessBusy(false);
    }
  };

  const handleLogout = () => {
    setCredentialNotice(null);
    void session.logout();
  };

  const accessPanel = (
    <ControlPlaneAccessPanel
      snapshot={snapshot}
      deviceId={deviceId}
      busy={accessBusy}
      credentialNotice={credentialNotice}
      onCredentialNotice={setCredentialNotice}
      onSubmit={handleAccess}
    />
  );

  if (snapshot.status === 'loading' && !accessBusy) {
    return (
      <main className="desktop-auth-gate" aria-live="polite">
        <Space direction="vertical" align="center" size="middle">
          <Spin size="large" />
          <Typography.Text type="secondary">正在校验桌面端会话…</Typography.Text>
        </Space>
      </main>
    );
  }

  if (snapshot.status === 'unauthenticated' || snapshot.status === 'activation_required' || accessBusy) {
    return (
      <Routes>
        <Route path="/login" element={accessPanel} />
        <Route path="*" element={<Navigate to="/login" replace />} />
      </Routes>
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
          extra={<Button type="primary" icon={<ReloadOutlined />} onClick={() => void session.restore(buildDeviceRegistration(deviceId))}>重新连接</Button>}
        />
      </main>
    );
  }

  if (!isConfirmedDesktopAccess(snapshot.user, snapshot.device, deviceId)) {
    return (
      <Routes>
        <Route path="/login" element={accessPanel} />
        <Route path="*" element={<Navigate to="/login" replace />} />
      </Routes>
    );
  }

  return (
    <>
      <ControlPlaneHeartbeat session={session} snapshot={snapshot} />
      {snapshot.warning || credentialNotice ? (
        <Space className="desktop-auth-session-warning" direction="vertical" size="small">
          {snapshot.warning ? <Alert type="warning" showIcon message={snapshot.warning} /> : null}
          {credentialNotice ? <Alert type={credentialNotice.type} showIcon message={credentialNotice.message} /> : null}
        </Space>
      ) : null}
      <div className="desktop-auth-session-controls">
        <Typography.Text type="secondary">{snapshot.user?.username ?? '已登录'}</Typography.Text>
        <AuthorizationExpiryStatus expiresAt={snapshot.device?.activation_expires_at} />
        <Button size="small" onClick={handleLogout}>退出登录</Button>
      </div>
      <Routes>
        <Route path="/login" element={<Navigate to="/" replace />} />
        <Route path="*" element={children} />
      </Routes>
    </>
  );
}

function publishCredentialNotice(
  successMessages: string[],
  warningMessages: string[],
  setNotice: (notice: CredentialNotice) => void,
) {
  if (warningMessages.length > 0) {
    setNotice({ type: 'warning', message: warningMessages.join(' ') });
    return;
  }
  if (successMessages.length > 0) {
    setNotice({ type: 'success', message: successMessages.join(' ') });
  }
}

function AuthorizationExpiryStatus({ expiresAt }: { expiresAt?: string | null }) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  if (!expiresAt) {
    return <Typography.Text className="desktop-auth-session-expiry" type="warning">账号授权剩余有效期：暂不可用</Typography.Text>;
  }
  const expiresAtMs = Date.parse(expiresAt);
  if (!Number.isFinite(expiresAtMs)) {
    return <Typography.Text className="desktop-auth-session-expiry" type="warning">账号授权有效期：暂不可用</Typography.Text>;
  }

  const remainingMs = expiresAtMs - now;
  const totalMinutes = Math.max(0, Math.ceil(remainingMs / 60_000));
  const days = Math.floor(totalMinutes / 1_440);
  const hours = Math.floor((totalMinutes % 1_440) / 60);
  const minutes = totalMinutes % 60;
  const remaining = remainingMs <= 0
    ? '已过期'
    : `${days > 0 ? `${days}天` : ''}${hours > 0 ? `${hours}小时` : ''}${minutes > 0 || (days === 0 && hours === 0) ? `${minutes}分钟` : ''}`;
  const type = remainingMs <= 0 ? 'danger' : remainingMs < 86_400_000 ? 'warning' : 'secondary';

  return (
    <Typography.Text className="desktop-auth-session-expiry" type={type} aria-live="polite">
      账号授权剩余有效期：{remaining}（{new Date(expiresAtMs).toLocaleString()} 到期）
    </Typography.Text>
  );
}
