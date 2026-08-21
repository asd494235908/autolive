import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('桌面端使用 hash 路由覆盖登录、主页、设置和状态页', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const shell = await readFile(new URL('./desktop/desktop-shell.tsx', import.meta.url), 'utf8');
  const gate = await readFile(new URL('./desktop/control-plane-gate.tsx', import.meta.url), 'utf8');

  assert.match(app, /<HashRouter>/);
  assert.match(app, /<ControlPlaneGate>/);
  assert.match(app, /<DesktopRouter \/>/);
  assert.match(app, /<Route path="\/" element={<DesktopApp \/>} \/>/);
  assert.match(app, /<Route path="\/settings" element={<DesktopApp \/>} \/>/);
  assert.match(app, /<Route path="\/status" element={<DesktopApp \/>} \/>/);
  assert.match(app, /<Route path="\*" element={<Navigate to="\/" replace \/>} \/>/);
  assert.match(gate, /<Route\s+path="\/login"\s+element={<LoginPanel/);
  assert.match(gate, /<Route path="\*" element={<Navigate to="\/login" replace \/>} \/>/);
  assert.match(gate, /<Route path="\/login" element={<Navigate to="\/" replace \/>} \/>/);
  assert.match(gate, /activation_expires_at/);
  assert.match(gate, /激活码剩余有效期：/);
  assert.match(gate, /setInterval\(\(\) => setNow\(Date\.now\(\)\), 60_000\)/);
  assert.match(gate, /<DesktopTopbar \/>/);
  assert.match(gate, /src="\/app-icon\.png"/);
  assert.match(gate, /欢迎登录/);
  assert.match(shell, /aria-label="桌面端页面导航"/);
  assert.match(shell, /path: '\/settings'/);
  assert.match(shell, /path: '\/status'/);
});

test('桌面端启动门禁使用控制面会话，不读取服务端加密主密钥', async () => {
  const gate = await readFile(new URL('./desktop/control-plane-gate.tsx', import.meta.url), 'utf8');
  const session = await readFile(new URL('./controlPlaneSession.ts', import.meta.url), 'utf8');

  assert.match(gate, /session\.restore\(deviceId\)/);
  assert.match(gate, /session\.login\(/);
  assert.match(gate, /session\.activate\(/);
  assert.match(gate, /session\.logout\(/);
  assert.match(session, /loadRefreshToken/);
  assert.match(session, /refreshControlPlane/);
  assert.match(session, /activateDeviceControlPlane/);
  assert.doesNotMatch(gate, /APP_SECRET_ENCRYPTION_KEY/);
  assert.doesNotMatch(session, /APP_SECRET_ENCRYPTION_KEY/);
});
