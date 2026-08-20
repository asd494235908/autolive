import { Card } from 'antd';
import type { ReactNode } from 'react';

export function DesktopPanel({
  title,
  extra,
  children,
  className = '',
}: {
  title: string;
  extra?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <Card
      className={`desktop-panel ${className}`.trim()}
      extra={extra}
      size="small"
      styles={{
        header: { minHeight: 44, padding: '0 12px' },
        body: {
          minHeight: 0,
          padding: 12,
          ...(className.includes('desktop-panel-fill')
            ? { height: 'calc(100% - 44px)', overflow: 'hidden auto' }
            : {}),
        },
      }}
      title={title}
    >
      {children}
    </Card>
  );
}
