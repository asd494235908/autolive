import { Badge, Card, Progress, Slider } from 'antd';

export type ParameterMetricCardProps = {
  label: string;
  value: number;
  baseline: number;
  min: number;
  max: number;
  step: number;
  unit: string;
  digits?: number;
  tone: 'blue' | 'green' | 'pink' | 'yellow';
  flashToken?: number;
  disabled?: boolean;
  onChange: (value: number) => void;
};

export function ParameterMetricCard({
  label,
  value,
  baseline,
  min,
  max,
  step,
  unit,
  digits = 2,
  tone,
  flashToken,
  disabled,
  onChange,
}: ParameterMetricCardProps) {
  const delta = value - baseline;
  const colors = {
    blue: '#5ea2ff',
    green: '#31d7aa',
    pink: '#f176af',
    yellow: '#f0bd3e',
  } as const;
  return (
    <Card
      className={getMetricCardClassName(flashToken)}
      size="small"
      styles={{ body: { height: 80, overflow: 'hidden', padding: '5px 10px 4px' } }}
    >
      <div className="parameter-metric-label"><Badge color={colors[tone]} />{label}</div>
      <strong className="parameter-metric-value">{value.toFixed(digits)}<span>{unit}</span></strong>
      <Slider
        aria-label={`${label}滑块`}
        disabled={disabled}
        min={min}
        max={max}
        step={step}
        tooltip={{ formatter: (next) => `${next ?? value}${unit}` }}
        value={value}
        style={{ margin: '1px 4px 0' }}
        onChange={onChange}
      />
      <small>基线 {baseline.toFixed(digits)}{unit} · 变化 {delta >= 0 ? '+' : ''}{delta.toFixed(digits)}</small>
    </Card>
  );
}

export type ReadOnlyMetricCardProps = {
  label: string;
  value: string;
  meta: string;
  tone: 'blue' | 'green' | 'pink' | 'yellow';
  percent: number;
  flashToken?: number;
};

function getMetricCardClassName(flashToken: number | undefined): string {
  return flashToken === undefined
    ? 'parameter-metric-card'
    : `parameter-metric-card parameter-metric-card-flash-${flashToken % 2}`;
}

export function ReadOnlyMetricCard({ label, value, meta, tone, percent, flashToken }: ReadOnlyMetricCardProps) {
  const colors = {
    blue: '#5ea2ff',
    green: '#31d7aa',
    pink: '#f176af',
    yellow: '#f0bd3e',
  } as const;
  return (
    <Card
      className={getMetricCardClassName(flashToken)}
      size="small"
      styles={{ body: { height: 80, overflow: 'hidden', padding: '5px 10px 4px' } }}
    >
      <div className="parameter-metric-label"><Badge color={colors[tone]} />{label}</div>
      <strong className="parameter-metric-value">{value}</strong>
      <Progress
        percent={Math.min(100, Math.max(0, percent))}
        showInfo={false}
        size="small"
        strokeColor={colors[tone]}
        style={{ margin: 0 }}
      />
      <small>{meta}</small>
    </Card>
  );
}
