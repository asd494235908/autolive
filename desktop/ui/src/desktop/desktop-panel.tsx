import { Card } from 'antd';
import type { ReactNode } from 'react';

export function DesktopPanel({
  title,
  titleIcon,
  subtitle,
  extra,
  children,
  className = '',
}: {
  title: ReactNode;
  titleIcon?: ReactNode;
  subtitle?: ReactNode;
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
        header: {
          minHeight: className.includes('desktop-media-domain-panel') ? 38 : 44,
          padding: '0 12px',
          ...(className.includes('desktop-media-domain-panel') ? { fontSize: 13 } : {}),
        },
        body: {
          minHeight: 0,
          padding: className.includes('desktop-media-domain-panel') || className.includes('desktop-compact-panel') ? 8 : 12,
          ...(className.includes('desktop-panel-fill')
            ? { height: 'calc(100% - 44px)', overflow: 'hidden auto' }
            : {}),
        },
      }}
      title={titleIcon || subtitle ? (
        <div className="desktop-panel-title">
          {titleIcon ? <span className="desktop-panel-title-icon" aria-hidden="true">{titleIcon}</span> : null}
          <span className="desktop-panel-title-copy">
            <span className="desktop-panel-title-text">{title}</span>
            {subtitle ? <span className="desktop-panel-title-subtitle">{subtitle}</span> : null}
          </span>
        </div>
      ) : title}
    >
      {children}
    </Card>
  );
}
