import React from 'react';
import ReactDOM from 'react-dom/client';
import { App as AntApp, ConfigProvider, theme } from 'antd';
import App from './App';

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <ConfigProvider
      theme={{
        algorithm: theme.darkAlgorithm,
        token: {
          colorBgBase: '#0b1220',
          colorBgLayout: '#0b1220',
          colorBgContainer: '#111c2e',
          colorBgElevated: '#16243a',
          colorText: '#f5f7fb',
          colorTextSecondary: '#c3cfdf',
          colorTextTertiary: '#9eacc0',
          colorTextHeading: '#ffffff',
          colorBorder: '#35506d',
          colorPrimary: '#22d3ee',
          colorFillAlter: 'rgba(255, 255, 255, 0.04)',
          fontSize: 16,
          controlHeight: 40,
        },
        components: {
          Typography: {
            titleMarginBottom: 12,
          },
          Card: {
            headerBg: '#16243a',
            extraColor: '#c3cfdf',
          },
          Descriptions: {
            labelColor: '#c3cfdf',
            contentColor: '#f5f7fb',
            titleColor: '#ffffff',
          },
          Form: {
            labelColor: '#c3cfdf',
          },
        },
      }}
    >
      <AntApp>
        <App />
      </AntApp>
    </ConfigProvider>
  </React.StrictMode>,
);
