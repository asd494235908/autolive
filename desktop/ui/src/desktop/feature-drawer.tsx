import { Card, Drawer, Typography } from 'antd';
import type { ReactNode } from 'react';

export function FeatureDrawer({
  title,
  description,
  width,
  open,
  onClose,
  summary,
  footer,
  children,
}: {
  title: string;
  description: string;
  width: number;
  open: boolean;
  onClose: () => void;
  summary: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Drawer
      rootClassName="feature-drawer"
      title={(
        <div className="feature-drawer-title">
          <strong>{title}</strong>
          <Typography.Text>{description}</Typography.Text>
        </div>
      )}
      width={`min(${width}px, 100vw)`}
      open={open}
      onClose={onClose}
      destroyOnHidden
      footer={footer ? <div className="feature-drawer-footer">{footer}</div> : undefined}
      styles={{
        header: { padding: '14px 16px', background: '#17171c', borderBottomColor: '#303139' },
        body: { padding: 12, background: '#0f0f14' },
        footer: { padding: '10px 16px', background: '#17171c', borderTopColor: '#303139' },
        mask: { background: 'rgba(0, 0, 0, 0.64)' },
      }}
    >
      <div className="feature-drawer-summary" aria-label={`${title}状态概览`} aria-live="polite">{summary}</div>
      <div className="feature-drawer-content">{children}</div>
    </Drawer>
  );
}

export function FeatureDrawerSection({
  title,
  description,
  extra,
  children,
}: {
  title: string;
  description?: string;
  extra?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Card
      className="feature-drawer-section"
      size="small"
      title={(
        <div className="feature-drawer-section-title">
          <h3>{title}</h3>
          {description ? <Typography.Text>{description}</Typography.Text> : null}
        </div>
      )}
      extra={extra}
      styles={{
        header: { minHeight: 46, padding: '0 12px' },
        body: { padding: 12 },
      }}
    >
      {children}
    </Card>
  );
}

export function FeatureDrawerField({
  label,
  htmlFor,
  hint,
  children,
}: {
  label: string;
  htmlFor?: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="feature-drawer-field">
      <label htmlFor={htmlFor}>{label}</label>
      {children}
      {hint ? <Typography.Text>{hint}</Typography.Text> : null}
    </div>
  );
}
