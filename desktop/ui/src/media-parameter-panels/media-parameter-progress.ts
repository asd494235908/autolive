export function normalizeMediaParameterProgress(
  value: number | null,
  minimum: number,
  maximum: number,
) {
  if (value === null || !Number.isFinite(value) || maximum <= minimum) return 0;
  return Math.min(100, Math.max(0, ((value - minimum) / (maximum - minimum)) * 100));
}
