import { Badge, Button, Card, Progress } from 'antd';

export type DesktopStatusItem = {
  key: string;
  label: string;
  value: string;
  meta: string;
  tone: 'green' | 'blue' | 'pink' | 'yellow';
  progress?: number;
  actionLabel?: string;
  onAction?: () => void;
};

export function DesktopStatusStrip({ items }: { items: readonly DesktopStatusItem[] }) {
  const colors = {
    green: '#31d7aa',
    blue: '#5ea2ff',
    pink: '#f176af',
    yellow: '#f0bd3e',
  } as const;
  return (
    <section className="desktop-status-strip" aria-label="实时状态概览">
      {items.map((item) => (
        <Card
          className={`desktop-status-card desktop-status-${item.tone}`}
          key={item.key}
          size="small"
          styles={{
            body: {
              height: 76,
              display: 'flex',
              flexDirection: 'column',
              gap: 1,
              overflow: 'hidden',
              padding: '5px 10px 4px',
            },
          }}
        >
          <div className="desktop-status-heading">
            <Badge color={colors[item.tone]} text={item.label} />
            {item.onAction && item.actionLabel ? (
              <Button size="small" type="text" style={{ height: 18, padding: '0 4px', fontSize: 10 }} onClick={item.onAction}>{item.actionLabel}</Button>
            ) : null}
          </div>
          <strong>{item.value}</strong>
          <small>{item.meta}</small>
          {typeof item.progress === 'number' ? (
            <Progress
              percent={Math.min(100, Math.max(0, item.progress))}
              showInfo={false}
              size="small"
              strokeColor={colors[item.tone]}
              style={{ margin: 0 }}
            />
          ) : null}
        </Card>
      ))}
    </section>
  );
}
