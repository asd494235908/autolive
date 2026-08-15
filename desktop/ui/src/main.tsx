import React, { lazy, Suspense } from 'react';
import ReactDOM from 'react-dom/client';
import { StartupErrorBoundary, StartupLoading } from './启动加载';

const App = lazy(() => import('./App'));

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <StartupErrorBoundary>
      <Suspense fallback={<StartupLoading message="正在加载界面…" />}>
        <App />
      </Suspense>
    </StartupErrorBoundary>
  </React.StrictMode>,
);
