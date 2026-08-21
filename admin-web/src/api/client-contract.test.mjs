import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('管理端对审计不可用只用同一幂等键重试一次', async () => {
  const source = await readFile(new URL('./client.ts', import.meta.url), 'utf8');

  assert.match(source, /allowAuditRetry = true/);
  assert.match(source, /response\.status === 503/);
  assert.match(source, /payload\.code === 'AUDIT_UNAVAILABLE'/);
  assert.match(source, /request<T>\(path, options, allowAuthRefresh, false\)/);
  assert.match(source, /name\.toLowerCase\(\) === 'idempotency-key'/);
});
