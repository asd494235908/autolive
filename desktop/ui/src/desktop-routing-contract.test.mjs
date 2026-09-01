import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('桌面端保留兼容路由，但顶部导航只展示主页', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const shell = await readFile(new URL('./desktop/desktop-shell.tsx', import.meta.url), 'utf8');
  const gate = await readFile(new URL('./desktop/control-plane-gate.tsx', import.meta.url), 'utf8');
  const accessPanel = await readFile(new URL('./desktop/control-plane-access-panel.tsx', import.meta.url), 'utf8');
  const main = await readFile(new URL('./main.tsx', import.meta.url), 'utf8');

  assert.match(app, /<HashRouter>/);
  assert.match(app, /<ControlPlaneGate>/);
  assert.match(app, /<DesktopRouter \/>/);
  assert.match(app, /<Route path="\/" element={<DesktopApp \/>} \/>/);
  assert.match(app, /<Route path="\/settings" element={<DesktopApp \/>} \/>/);
  assert.match(app, /<Route path="\/status" element={<DesktopApp \/>} \/>/);
  assert.match(app, /<Route path="\*" element={<Navigate to="\/" replace \/>} \/>/);
  assert.match(gate, /<Route path="\/login" element=\{accessPanel\}/);
  assert.match(gate, /<ControlPlaneAccessPanel/);
  assert.match(gate, /<Route path="\*" element={<Navigate to="\/login" replace \/>} \/>/);
  assert.match(gate, /<Route path="\/login" element={<Navigate to="\/" replace \/>} \/>/);
  assert.match(gate, /activation_expires_at/);
  assert.match(gate, /账号授权剩余有效期：/);
  assert.match(gate, /<Badge className="desktop-auth-session-authorization" status="success" text="设备已授权" \/>/);
  assert.match(gate, /snapshot\.user\?\.username/);
  assert.doesNotMatch(gate, /kangyun|364\s*天|设备\s*1\/3/);
  assert.match(gate, /setInterval\(\(\) => setNow\(Date\.now\(\)\), 60_000\)/);
  assert.match(gate, /<Spin size="large" \/>/);
  assert.match(gate, /正在校验桌面端会话…/);
  assert.doesNotMatch(gate, /<Spin[^>]*\btip=/);
  assert.doesNotMatch(accessPanel, /DesktopTopbar/);
  assert.match(accessPanel, /src="\/app-icon\.png"/);
  assert.match(accessPanel, /账号登录/);
  assert.match(accessPanel, /普通用户账号和密码；管理员账号请在管理端登录/);
  assert.doesNotMatch(accessPanel, /激活码|activationCode/);
  assert.match(shell, /aria-label="桌面端页面导航"/);
  assert.match(shell, /const DESKTOP_ROUTE_ITEMS = \[\s*\{ path: '\/', label: '主页' \},\s*\] as const/);
  assert.doesNotMatch(shell, /path: '\/settings'/);
  assert.doesNotMatch(shell, /path: '\/status'/);
  assert.match(shell, /export function DesktopWindowFrame/);
  assert.doesNotMatch(shell.slice(shell.indexOf('export function DesktopShell')), /<DesktopTopbar \/>/);
  assert.match(main, /<DesktopWindowFrame>/);
});

test('桌面端启动门禁使用控制面会话，不读取服务端加密主密钥', async () => {
  const gate = await readFile(new URL('./desktop/control-plane-gate.tsx', import.meta.url), 'utf8');
  const session = await readFile(new URL('./controlPlaneSession.ts', import.meta.url), 'utf8');

  assert.match(gate, /session\.restore\(buildDeviceRegistration\(stableDeviceId\)\)/);
  assert.match(gate, /session\.login\(/);
  assert.doesNotMatch(gate, /session\.activate\(/);
  assert.match(gate, /session\.logout\(/);
  assert.match(session, /restoreControlPlaneSession/);
  assert.match(session, /refreshControlPlaneSession/);
  assert.match(session, /activateDeviceControlPlane/);
  assert.doesNotMatch(session, /refreshToken|refresh_token/);
  assert.doesNotMatch(session, /activation_code\s*:/);
  assert.doesNotMatch(gate, /APP_SECRET_ENCRYPTION_KEY/);
  assert.doesNotMatch(session, /APP_SECRET_ENCRYPTION_KEY/);
});
