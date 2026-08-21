import { BorderOutlined, CloseOutlined, MinusOutlined } from '@ant-design/icons';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Button, Space } from 'antd';
import type { ReactNode } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

function runWindowAction(action: 'minimize' | 'toggleMaximize' | 'close') {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
  const appWindow = getCurrentWindow();
  void appWindow[action]().catch(() => undefined);
}

export function DesktopTopbar() {
  return (
    <header className="desktop-topbar" data-tauri-drag-region>
      <div className="desktop-brand" data-tauri-drag-region>
        <img className="desktop-brand-icon" src="/app-icon.png" alt="" data-tauri-drag-region />
        <span className="desktop-brand-name" data-tauri-drag-region>autoLive</span>
        <span className="desktop-brand-subtitle" data-tauri-drag-region>本地单视频循环引擎</span>
      </div>
      <DesktopWindowControls />
    </header>
  );
}

export function DesktopWindowControls() {
  return (
    <div className="desktop-window-controls">
      <Button
        aria-label="最小化窗口"
        className="desktop-window-button"
        icon={<MinusOutlined />}
        onClick={() => runWindowAction('minimize')}
        type="text"
      />
      <Button
        aria-label="最大化或还原窗口"
        className="desktop-window-button"
        icon={<BorderOutlined />}
        onClick={() => runWindowAction('toggleMaximize')}
        type="text"
      />
      <Button
        aria-label="关闭窗口"
        className="desktop-window-button desktop-window-button-close"
        icon={<CloseOutlined />}
        onClick={() => runWindowAction('close')}
        type="text"
      />
    </div>
  );
}

const DESKTOP_ROUTE_ITEMS = [
  { path: '/', label: '主页' },
  { path: '/settings', label: '设置' },
  { path: '/status', label: '状态' },
] as const;

function DesktopRouteNavigation() {
  const location = useLocation();
  const navigate = useNavigate();
  const currentPath = DESKTOP_ROUTE_ITEMS.some((item) => item.path === location.pathname)
    ? location.pathname
    : '/';

  return (
    <nav className="desktop-route-nav" aria-label="桌面端页面导航">
      <span className="desktop-route-nav-label">工作区</span>
      <Space size={4}>
        {DESKTOP_ROUTE_ITEMS.map((item) => (
          <Button
            key={item.path}
            size="small"
            type={currentPath === item.path ? 'primary' : 'text'}
            aria-current={currentPath === item.path ? 'page' : undefined}
            onClick={() => navigate(item.path)}
          >
            {item.label}
          </Button>
        ))}
      </Space>
    </nav>
  );
}

export function DesktopShell({ children }: { children: ReactNode }) {
  return (
    <div className="desktop-page">
      <DesktopTopbar />
      <DesktopRouteNavigation />
      <main className="desktop-page-content">
        <div className="desktop-workspace">{children}</div>
      </main>
    </div>
  );
}

export function DesktopColumn({
  area,
  ariaLabel,
  children,
}: {
  area: 'source' | 'video' | 'audio-output';
  ariaLabel: string;
  children: ReactNode;
}) {
  return (
    <section className={`desktop-column desktop-column-${area}`} aria-label={ariaLabel}>
      {children}
    </section>
  );
}
