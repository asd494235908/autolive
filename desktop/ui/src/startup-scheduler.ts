type IdleCallback = () => void;

type IdleScheduler = typeof globalThis & {
  requestIdleCallback?: (callback: IdleCallback, options?: { timeout: number }) => number;
  cancelIdleCallback?: (handle: number) => void;
};

export function waitForAbortableDelay(delayMs: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    let timer: ReturnType<typeof globalThis.setTimeout> | undefined;
    const cleanup = () => {
      if (timer !== undefined) globalThis.clearTimeout(timer);
      signal?.removeEventListener('abort', onAbort);
    };
    const onAbort = () => {
      cleanup();
      reject(new DOMException('The operation was aborted', 'AbortError'));
    };

    if (signal?.aborted) {
      onAbort();
      return;
    }

    signal?.addEventListener('abort', onAbort, { once: true });
    if (signal?.aborted) {
      onAbort();
      return;
    }
    timer = globalThis.setTimeout(() => {
      cleanup();
      resolve();
    }, Math.max(0, delayMs));
  });
}

export function scheduleAfterInitialPaint(task: IdleCallback, timeoutMs = 500): () => void {
  const scheduler = globalThis as IdleScheduler;
  if (scheduler.requestIdleCallback) {
    const handle = scheduler.requestIdleCallback(task, { timeout: timeoutMs });
    return () => scheduler.cancelIdleCallback?.(handle);
  }

  const timer = globalThis.setTimeout(task, 0);
  return () => globalThis.clearTimeout(timer);
}
