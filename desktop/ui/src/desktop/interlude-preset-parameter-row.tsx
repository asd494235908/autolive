import { memo } from 'react';

export const InterludePresetParameterRow = memo(function InterludePresetParameterRow({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="desktop-preset-parameter-row" title={`${label}：${value}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
});
