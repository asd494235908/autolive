export const ACTIVATION_CODE_EXPIRY_PRESETS = [
  { label: '3天', amount: 3, unit: 'day' },
  { label: '7天', amount: 7, unit: 'day' },
  { label: '30天', amount: 30, unit: 'day' },
  { label: '90天', amount: 90, unit: 'day' },
  { label: '1年', amount: 1, unit: 'year' }
] as const;

export type ActivationCodeExpiryUnit = (typeof ACTIVATION_CODE_EXPIRY_PRESETS)[number]['unit'];

export function calculateActivationCodeExpiry(
  now: Date,
  amount: number,
  unit: ActivationCodeExpiryUnit
): Date {
  if (Number.isNaN(now.getTime())) {
    throw new Error('有效期计算需要有效的当前时间');
  }

  if (!Number.isInteger(amount) || amount <= 0) {
    throw new Error('有效期时长必须是正整数');
  }

  const expiry = new Date(now.getTime());
  if (unit === 'year') {
    expiry.setFullYear(expiry.getFullYear() + amount);
  } else {
    expiry.setDate(expiry.getDate() + amount);
  }

  return expiry;
}
