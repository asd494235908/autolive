import { Component, type CSSProperties, type ReactNode } from 'react';

const shellStyle: CSSProperties = {
  alignItems: 'center',
  background: '#fff',
  color: 'rgba(0, 0, 0, 0.88)',
  display: 'flex',
  flexDirection: 'column',
  fontFamily: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
  gap: 12,
  inset: 0,
  justifyContent: 'center',
  minHeight: '100vh',
  padding: 24,
  position: 'fixed',
  textAlign: 'center',
};

const spinnerStyle: CSSProperties = {
  animation: 'startup-loading-spin 0.8s linear infinite',
  border: '3px solid rgba(0, 0, 0, 0.06)',
  borderRadius: '50%',
  borderTopColor: '#1677ff',
  height: 28,
  width: 28,
};

export function StartupLoading({ message = '正在启动桌面端…' }: { message?: string }) {
  return (
    <main aria-live="polite" role="status" style={shellStyle}>
      <style>{'@keyframes startup-loading-spin { to { transform: rotate(360deg); } }'}</style>
      <span aria-hidden="true" style={spinnerStyle} />
      <span>{message}</span>
    </main>
  );
}

type StartupErrorBoundaryProps = {
  children: ReactNode;
};

type StartupErrorBoundaryState = {
  error: Error | null;
};

export class StartupErrorBoundary extends Component<StartupErrorBoundaryProps, StartupErrorBoundaryState> {
  state: StartupErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: unknown): StartupErrorBoundaryState {
    return {
      error: error instanceof Error ? error : new Error('应用启动时发生未知错误'),
    };
  }

  handleReload = () => {
    window.location.reload();
  };

  render() {
    if (!this.state.error) return this.props.children;

    return (
      <main role="alert" style={shellStyle}>
        <h1 style={{ fontSize: 20, margin: 0 }}>启动失败</h1>
        <p style={{ color: 'rgba(0, 0, 0, 0.65)', margin: 0, maxWidth: 520 }}>{this.state.error.message}</p>
        <button
          onClick={this.handleReload}
          style={{
            background: '#1677ff',
            border: '1px solid #1677ff',
            borderRadius: 6,
            color: '#fff',
            cursor: 'pointer',
            padding: '8px 16px',
          }}
          type="button"
        >
          重新加载
        </button>
      </main>
    );
  }
}
