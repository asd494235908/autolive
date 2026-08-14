import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./heartbeatOutbox.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const {
  clearHeartbeatOutbox,
  queueLatestHeartbeat,
  readHeartbeatOutbox,
  shouldQueueHeartbeat,
} = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

function createStorage() {
  const values = new Map();
  return {
    getItem(key) {
      return values.has(key) ? values.get(key) : null;
    },
    setItem(key, value) {
      values.set(key, value);
    },
    removeItem(key) {
      values.delete(key);
    },
  };
}

const heartbeat = {
  device_id: 'device-1',
  sent_at: '2026-08-13T00:00:00Z',
  status: { disk_free_bytes: 10, playback_state: 'playing' },
};

test('heartbeat outbox keeps the latest request scoped by user and device', () => {
  const storage = createStorage();
  const entry = queueLatestHeartbeat(storage, 'user-1', 'device-1', heartbeat, 'idem-1', 'queued-1');
  assert.equal(entry.queued_at, 'queued-1');
  assert.deepEqual(readHeartbeatOutbox(storage, 'user-1', 'device-1'), entry);
  assert.equal(readHeartbeatOutbox(storage, 'user-2', 'device-1'), null);

  const newer = queueLatestHeartbeat(
    storage,
    'user-1',
    'device-1',
    { ...heartbeat, sent_at: '2026-08-13T00:01:00Z' },
    'idem-2',
    'queued-2',
  );
  assert.equal(readHeartbeatOutbox(storage, 'user-1', 'device-1').idempotency_key, 'idem-2');
  assert.equal(newer.queued_at, 'queued-2');
});

test('heartbeat outbox clears only the matching idempotency key', () => {
  const storage = createStorage();
  queueLatestHeartbeat(storage, 'user-1', 'device-1', heartbeat, 'idem-1', 'queued-1');
  clearHeartbeatOutbox(storage, 'user-1', 'device-1', 'other-key');
  assert.notEqual(readHeartbeatOutbox(storage, 'user-1', 'device-1'), null);
  clearHeartbeatOutbox(storage, 'user-1', 'device-1', 'idem-1');
  assert.equal(readHeartbeatOutbox(storage, 'user-1', 'device-1'), null);
});

test('only transient control-plane failures are queued', () => {
  assert.equal(shouldQueueHeartbeat(new Error('network offline')), true);
  assert.equal(shouldQueueHeartbeat({ status: 408 }), true);
  assert.equal(shouldQueueHeartbeat({ status: 503 }), true);
  assert.equal(shouldQueueHeartbeat({ status: 401 }), false);
  assert.equal(shouldQueueHeartbeat({ status: 422 }), false);
});
