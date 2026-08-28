import { Alert, Button, Space, Typography } from "antd";

export function InlineRecoveryAlert({
  issue,
  recovering = false,
  onRecover,
  onSecondary,
}) {
  if (!issue) return null;

  return (
    <Alert
      className="inline-recovery-alert"
      type={issue.type ?? "error"}
      showIcon
      role="alert"
      message={recovering ? `正在恢复：${issue.title}` : issue.title}
      description={(
        <div className="inline-recovery-alert__content">
          <Typography.Text>{recovering ? issue.recoveringDescription : issue.description}</Typography.Text>
          <Space size={6} wrap>
            <Button size="small" type="primary" loading={recovering} disabled={recovering} onClick={onRecover}>
              {recovering ? "恢复中" : issue.recoverLabel}
            </Button>
            {issue.secondaryLabel ? (
              <Button size="small" disabled={recovering} onClick={onSecondary}>{issue.secondaryLabel}</Button>
            ) : null}
          </Space>
        </div>
      )}
    />
  );
}

export default InlineRecoveryAlert;
