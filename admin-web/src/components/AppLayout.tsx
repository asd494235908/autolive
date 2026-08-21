import {
  DesktopOutlined,
  FileSearchOutlined,
  KeyOutlined,
  PartitionOutlined,
  ApiOutlined,
  SafetyCertificateOutlined,
  SettingOutlined,
  TeamOutlined,
} from '@ant-design/icons';
import { Button, Layout, Menu, Space, Typography } from 'antd';
import type { PropsWithChildren, ReactNode } from 'react';
import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { apiClient } from '../api/client';
import { clearSession, readSession } from '../features/auth/session';
import type { LogoutResponse } from '../types/api';

const { Header, Sider, Content } = Layout;

type NavItem = {
  key: string;
  label: string;
  path: string;
};

const iconMap: Record<string, ReactNode> = {
  dashboard: <DesktopOutlined />,
  users: <TeamOutlined />,
  devices: <DesktopOutlined />,
  'activation-codes': <KeyOutlined />,
  'model-pools': <PartitionOutlined />,
  'model-leases': <ApiOutlined />,
  'audit-logs': <FileSearchOutlined />,
  security: <SettingOutlined />
};

export function AppLayout({
  children,
  currentPath,
  navItems
}: PropsWithChildren<{
  currentPath: string;
  navItems: NavItem[];
}>) {
  const navigate = useNavigate();
  const [logoutPending, setLogoutPending] = useState(false);

  const selectedKey = useMemo(() => {
    const matched = navItems.find((item) => item.path === currentPath);
    return matched?.key ?? 'dashboard';
  }, [currentPath, navItems]);

  return (
    <Layout style={{ minHeight: '100vh' }}>
      <Sider breakpoint="lg" collapsedWidth="0">
        <div style={{ padding: 16 }}>
          <Typography.Title level={4} style={{ color: '#fff', margin: 0 }}>
            AutoLive
          </Typography.Title>
          <Typography.Text style={{ color: 'rgba(255,255,255,0.65)' }}>
            管理端骨架
          </Typography.Text>
        </div>

        <Menu
          theme="dark"
          mode="inline"
          selectedKeys={[selectedKey]}
          items={navItems.map((item) => ({
            key: item.key,
            icon: iconMap[item.key] ?? <SafetyCertificateOutlined />,
            label: item.label,
            onClick: () => navigate(item.path)
          }))}
        />
      </Sider>

      <Layout>
        <Header
          style={{
            background: '#fff',
            paddingInline: 24,
            display: 'flex',
            alignItems: 'center'
          }}
        >
          <Space style={{ width: '100%', justifyContent: 'space-between' }}>
            <Typography.Title level={4} style={{ margin: 0 }}>
              AutoLive 管理系统
            </Typography.Title>
            <Space>
              <Typography.Text>{readSession()?.user.username ?? '未登录'}</Typography.Text>
              <Button
                type="link"
                loading={logoutPending}
                onClick={() => {
                  setLogoutPending(true);
                  const session = readSession();
                  void apiClient
                    .post<LogoutResponse>('/api/v1/auth/logout', {
                      body: session?.tokens.refresh_token
                        ? { refresh_token: session.tokens.refresh_token }
                        : undefined
                    })
                    .catch(() => undefined)
                    .finally(() => {
                      clearSession();
                      navigate('/login', { replace: true });
                      setLogoutPending(false);
                    });
                }}
              >
                退出登录
              </Button>
            </Space>
          </Space>
        </Header>

        <Content style={{ margin: 24 }}>
          <div
            style={{
              minHeight: 'calc(100vh - 112px)',
              background: '#fff',
              borderRadius: 8,
              padding: 24
            }}
          >
            {children}
          </div>
        </Content>
      </Layout>
    </Layout>
  );
}
