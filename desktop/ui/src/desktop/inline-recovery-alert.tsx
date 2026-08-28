import { Alert, Button, Space } from 'antd';

export function InlineRecoveryAlert({
  message,
  description,
  actionLabel = '重试',
  busy = false,
  onAction,
}: {
  message: string | null | undefined;
  description?: string;
  actionLabel?: string;
  busy?: boolean;
  onAction?: () => void;
}) {
  if (!message) return null;
  return (
    <Alert
      className="desktop-inline-alert"
      type="error"
      showIcon
      message={message}
      description={description}
      action={onAction ? <Space><Button size="small" loading={busy} onClick={onAction}>{actionLabel}</Button></Space> : undefined}
    />
  );
}
