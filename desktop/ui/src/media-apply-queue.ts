export type PendingApplyRef<T> = { current: T | null };

export async function flushLatestPendingApply<T>(
  pendingRef: PendingApplyRef<T>,
  isBlocked: () => boolean,
  apply: (pending: T) => Promise<void>,
): Promise<boolean> {
  const pending = pendingRef.current;
  if (pending === null || isBlocked()) return false;
  pendingRef.current = null;
  await apply(pending);
  return true;
}
