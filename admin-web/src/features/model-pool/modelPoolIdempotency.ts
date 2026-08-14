import type { CreateModelPoolAccountRequest } from '../../types/api';

function fingerprint(values: CreateModelPoolAccountRequest) {
  return JSON.stringify([
    values.provider,
    values.model,
    values.api_key,
    values.priority,
    values.daily_limit,
    values.concurrency_limit
  ]);
}

export function createModelPoolIdempotencyKeyManager(createKey: () => string) {
  let activeFingerprint: string | undefined;
  let activeKey: string | undefined;

  return {
    getKey(values: CreateModelPoolAccountRequest) {
      const nextFingerprint = fingerprint(values);
      if (nextFingerprint !== activeFingerprint || activeKey === undefined) {
        activeFingerprint = nextFingerprint;
        activeKey = createKey();
      }

      return activeKey;
    },
    clear() {
      activeFingerprint = undefined;
      activeKey = undefined;
    }
  };
}
