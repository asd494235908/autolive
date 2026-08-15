export function getDisplayErrorMessage(cause: unknown, fallback: string): string {
  if (cause instanceof Error) {
    const message = cause.message.trim();
    return message || fallback;
  }
  if (typeof cause === 'string') {
    const message = cause.trim();
    return message || fallback;
  }
  if (cause && typeof cause === 'object' && 'message' in cause) {
    const message = (cause as { message?: unknown }).message;
    if (typeof message === 'string' && message.trim()) return message.trim();
  }
  return fallback;
}
