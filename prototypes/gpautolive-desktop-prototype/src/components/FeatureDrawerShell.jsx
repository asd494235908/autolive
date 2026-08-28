import { Card, Drawer, Typography } from "antd";

const DRAWER_STYLES = {
  header: { flex: "0 0 auto", padding: "14px 16px", background: "#17171c", borderBottomColor: "#303139" },
  body: { minHeight: 0, padding: 12, overflowY: "auto", background: "#0f0f14" },
  footer: { flex: "0 0 auto", padding: "10px 16px", background: "#17171c", borderTopColor: "#303139" },
  mask: { background: "rgba(0, 0, 0, 0.64)" },
};

export function FeatureDrawerShell({
  modifierClass,
  title,
  description,
  width = "min(560px, 100vw)",
  open,
  onClose,
  summary,
  summaryLabel,
  footer,
  children,
}) {
  return (
    <Drawer
      rootClassName={`feature-drawer ${modifierClass}`}
      title={(
        <div className="feature-drawer__title">
          <strong>{title}</strong>
          <Typography.Text>{description}</Typography.Text>
        </div>
      )}
      width={width}
      open={open}
      onClose={onClose}
      autoFocus
      keyboard
      destroyOnHidden
      footer={<div className="feature-drawer__footer">{footer}</div>}
      styles={DRAWER_STYLES}
    >
      {summary ? (
        <div className="feature-drawer__summary" aria-label={summaryLabel} aria-live="polite">
          {summary}
        </div>
      ) : null}
      <div className="feature-drawer__content">{children}</div>
    </Drawer>
  );
}

export function FeatureDrawerSection({ title, description, extra, children, className = "" }) {
  return (
    <Card
      className={`feature-drawer__section ${className}`.trim()}
      size="small"
      title={(
        <div className="feature-drawer__section-title">
          <h3>{title}</h3>
          {description ? <Typography.Text>{description}</Typography.Text> : null}
        </div>
      )}
      extra={extra}
      styles={{
        header: { minHeight: 46, padding: "0 12px" },
        body: { padding: 12 },
      }}
    >
      {children}
    </Card>
  );
}

export default FeatureDrawerShell;
