import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('桌面会话启动恢复和刷新使用代际保护及系统钥匙串', async () => {
  const source = await readFile(new URL('./controlPlaneSession.ts', import.meta.url), 'utf8');

  assert.match(source, /loadRefreshToken/);
  assert.match(source, /refreshControlPlane/);
  assert.match(source, /storeRefreshToken/);
  assert.match(source, /deleteRefreshToken/);
  assert.match(source, /activateDeviceControlPlane/);
  assert.match(source, /generation = 0/);
  assert.match(source, /if \(!this\.isCurrent\(generation\)\) return null/);
  assert.match(source, /this\.publish\(localSnapshot\)/);
  assert.match(source, /await this\.clearStoredCredential\(deviceId\)/);
  assert.match(source, /系统钥匙串不可用/);
  assert.match(source, /clearSessionRefreshHandler/);
});

test('已激活桌面会话按固定周期上报运行信息并限制心跳 outbox', async () => {
  const source = await readFile(new URL('./desktop/control-plane-heartbeat.tsx', import.meta.url), 'utf8');

  assert.match(source, /get_device_runtime_info/);
  assert.match(source, /HEARTBEAT_INTERVAL_MS = 30_000/);
  assert.match(source, /readHeartbeatOutbox/);
  assert.match(source, /queueLatestHeartbeat/);
  assert.match(source, /clearHeartbeatOutbox/);
  assert.match(source, /sendHeartbeatControlPlane/);
  assert.doesNotMatch(source, /refresh_token|access_token.*localStorage/);
});
