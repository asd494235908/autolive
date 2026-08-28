import { getCurrentWindow } from '@tauri-apps/api/window';
import { App as AntApp, ConfigProvider, theme as antdTheme } from 'antd';
import React, { lazy, Suspense } from 'react';
import ReactDOM from 'react-dom/client';
import { getCspNonce } from './cspNonce';
import { DesktopWindowFrame } from './desktop/desktop-shell';
import { StartupErrorBoundary, StartupLoading } from './startup-loader';
import './desktop-layout.css';

const App = lazy(() => import('./App'));

const isFinalEffectWindow =
  typeof window !== 'undefined'
  && '__TAURI_INTERNALS__' in window
  && getCurrentWindow().label === 'final-effect';

const content = (
  <StartupErrorBoundary>
    <Suspense fallback={<StartupLoading message="正在加载界面…" />}>
      <App />
    </Suspense>
  </StartupErrorBoundary>
);

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <ConfigProvider
      csp={{ nonce: getCspNonce() }}
      theme={isFinalEffectWindow ? undefined : {
        algorithm: antdTheme.darkAlgorithm,
        token: {
          colorPrimary: '#31d7aa',
          colorInfo: '#5ea2ff',
          colorBgBase: '#0b0b0f',
          colorBgContainer: '#17171c',
          colorBorder: '#303139',
          borderRadius: 8,
          fontSize: 12,
          controlHeight: 30,
        },
      }}
    >
      <AntApp message={isFinalEffectWindow ? undefined : { top: 44 }}>
        {isFinalEffectWindow ? content : <DesktopWindowFrame>{content}</DesktopWindowFrame>}
      </AntApp>
    </ConfigProvider>
  </React.StrictMode>,
);
