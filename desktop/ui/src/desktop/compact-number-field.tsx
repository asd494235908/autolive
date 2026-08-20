import { InputNumber, Typography } from 'antd';

export type CompactNumberFieldProps = {
  ariaLabel: string;
  label?: string;
  unit?: string;
  min: number;
  max: number;
  step?: number;
  value: number | null;
  onChange: (value: number | null) => void;
  onBlur?: () => void;
};

export function CompactNumberField({
  ariaLabel,
  label,
  unit,
  min,
  max,
  step,
  value,
  onChange,
  onBlur,
}: CompactNumberFieldProps) {
  return (
    <div className="desktop-number-field">
      {label ? <Typography.Text className="desktop-number-label">{label}</Typography.Text> : null}
      <InputNumber
        aria-label={ariaLabel}
        min={min}
        max={max}
        step={step}
        value={value}
        style={{ width: '100%' }}
        onChange={onChange}
        onBlur={onBlur}
      />
      {unit ? <Typography.Text className="desktop-number-unit">{unit}</Typography.Text> : null}
    </div>
  );
}
