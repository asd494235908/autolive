export type MetricValueSnapshot = Readonly<Record<string, number | string | null>>;

export function getChangedMetricKeys(
  previous: MetricValueSnapshot | null,
  current: MetricValueSnapshot,
): string[] {
  if (previous === null) return [];
  return Object.keys(current).filter((key) => !Object.is(previous[key], current[key]));
}

export function getNewlyActivePresetIds(
  previous: readonly string[] | null,
  current: readonly string[],
): string[] {
  if (previous === null) return [];
  const previousIds = new Set(previous);
  return [...new Set(current)].filter((id) => !previousIds.has(id));
}

export function advanceMetricFlashTokens(
  current: Readonly<Record<string, number>>,
  changedKeys: readonly string[],
): Record<string, number> {
  if (changedKeys.length === 0) return current;
  const next = { ...current };
  for (const key of new Set(changedKeys)) next[key] = (next[key] ?? 0) + 1;
  return next;
}
