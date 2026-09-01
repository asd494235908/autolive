import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('桌面会话启动恢复和刷新使用代际保护，Refresh Token 仅由 Rust 持有', async () => {
  const source = await readFile(new URL('./controlPlaneSession.ts', import.meta.url), 'utf8');

  assert.match(source, /restoreControlPlaneSession/);
  assert.match(source, /refreshControlPlaneSession/);
  assert.match(source, /loginControlPlaneSession/);
  assert.match(source, /logoutControlPlaneSession/);
  assert.doesNotMatch(source, /refreshToken|refresh_token|loadRefreshToken|storeRefreshToken/);
  assert.match(source, /generation = 0/);
  assert.match(source, /if \(!this\.isCurrent\(generation\)\) return null/);
  assert.match(source, /this\.publish\(localSnapshot\)/);
  assert.match(source, /clearSessionRefreshHandler/);
});

test('桌面会话在 Profile 表明未绑定时自动注册设备且不接收激活码', async () => {
  const source = await readFile(new URL('./controlPlaneSession.ts', import.meta.url), 'utf8');

  assert.match(source, /activateDeviceControlPlane/);
  assert.match(source, /await getClientProfileControlPlane\(accessToken\)[\s\S]*activateDeviceControlPlane/);
  assert.match(source, /let registrationAttempted = false[\s\S]*registrationAttempted = true[\s\S]*!registrationAttempted/);
  assert.match(source, /device:\s*this\.deviceRegistration/);
  assert.doesNotMatch(source, /activation_code\s*:/);
  assert.doesNotMatch(source, /async activate\(/);
});

test('旧设备待授权或授权过期时也会尝试自动重绑', async () => {
  const source = await readFile(new URL('./controlPlaneSession.ts', import.meta.url), 'utf8');

  assert.match(source, /shouldAutomaticallyRegisterDevice\(profile, this\.deviceId\)/);
  assert.match(source, /activateDeviceControlPlane/);
});

test('桌面会话只在服务端用户、当前设备与授权有效期均确认后进入 ready', async () => {
  const source = await readFile(new URL('./controlPlaneSession.ts', import.meta.url), 'utf8');

  assert.match(source, /isConfirmedDesktopAccess/);
  assert.match(source, /profile\.user/);
  assert.match(source, /profile\.device/);
  assert.match(source, /scheduleActivationRecheck/);
  assert.match(source, /setTimeout/);
  assert.match(source, /clearActivationRecheck/);
});

test('桌面登录区分凭据类 4xx 与网络、配置或服务端故障', async () => {
  const source = await readFile(new URL('./controlPlaneSession.ts', import.meta.url), 'utf8');

  assert.match(source, /function isLoginFormError/);
  assert.match(source, /status === 400 \|\| status === 401 \|\| status === 429/);
  assert.match(source, /status: isLoginFormError\(error\) \? 'unauthenticated' : 'error'/);
});

test('已激活桌面会话按固定周期上报运行信息并限制心跳 outbox', async () => {
  const source = await readFile(new URL('./desktop/control-plane-heartbeat.tsx', import.meta.url), 'utf8');

  assert.match(source, /get_device_runtime_info/);
  assert.match(source, /HEARTBEAT_INTERVAL_MS = 30_000/);
  assert.match(source, /readHeartbeatOutbox/);
  assert.match(source, /queueLatestHeartbeat/);
  assert.match(source, /clearHeartbeatOutbox/);
  assert.match(source, /sendHeartbeatControlPlane/);
  assert.match(source, /product:\s*'autolive'/);
  assert.doesNotMatch(source, /refresh_token|access_token.*localStorage/);
});

test('最终效果窗口只获得必需播放 IPC，不获得认证、HTTP、文件选择或无关命令', async () => {
  const [build, main, mainCapability, finalCapability, permissionSets] = await Promise.all([
    readFile(new URL('../../src-tauri/build.rs', import.meta.url), 'utf8'),
    readFile(new URL('../../src-tauri/src/main.rs', import.meta.url), 'utf8'),
    readFile(new URL('../../src-tauri/capabilities/default.json', import.meta.url), 'utf8'),
    readFile(new URL('../../src-tauri/capabilities/final-effect.json', import.meta.url), 'utf8'),
    readFile(new URL('../../src-tauri/permissions/command-sets.toml', import.meta.url), 'utf8'),
  ]);
  const finalSet = permissionSets.slice(permissionSets.indexOf('identifier = "final-effect-commands"'));

  assert.match(build, /AppManifest::new\(\)\.commands\(commands\)/);
  assert.match(build, /"login_control_plane_session"/);
  const registered = main.match(/tauri::generate_handler!\[([\s\S]*?)\]\)/)?.[1]
    .split(',').map((value) => value.trim()).filter(Boolean).sort();
  const declared = build.match(/let commands = &\[([\s\S]*?)\];/)?.[1]
    .match(/"([a-z0-9_]+)"/g).map((value) => value.slice(1, -1)).sort();
  assert.deepEqual(declared, registered);
  assert.match(mainCapability, /"main-commands"/);
  assert.match(finalCapability, /"final-effect-commands"/);
  assert.doesNotMatch(finalCapability, /http:|dialog:|main-commands/);
  for (const command of [
    'allow-get-snapshot',
    'allow-complete-playback-item',
    'allow-commit-media-processing-if-ready',
    'allow-release-media-processing-artifact',
    'allow-resize-final-effect-window',
  ]) {
    assert.match(finalSet, new RegExp(`"${command}"`));
  }
  assert.doesNotMatch(finalSet, /auth|control-plane|probe-local|runtime-resource|direct-model-chat|open-final-effect-window/);
});

test('退出登录先尝试远端撤销，断网时仅在 Rust 钥匙串保留待重试凭据', async () => {
  const [rust, bridge, session] = await Promise.all([
    readFile(new URL('../../src-tauri/src/control_plane_auth.rs', import.meta.url), 'utf8'),
    readFile(new URL('./controlPlaneAuth.ts', import.meta.url), 'utf8'),
    readFile(new URL('./controlPlaneSession.ts', import.meta.url), 'utf8'),
  ]);

  assert.match(rust, /pending-logout-/);
  assert.match(rust, /revoke_remote_token\(&token\)[\s\S]*store_pending_logout_token/);
  assert.match(rust, /retry_pending_control_plane_logout/);
  assert.match(bridge, /retry_pending_control_plane_logout/);
  assert.doesNotMatch(bridge, /pending-logout|refresh_token|refreshToken/);
  assert.match(session, /retryPendingControlPlaneLogout/);
  assert.match(session, /本机已退出|远端撤销/);
});

test('Rust 串行执行同一钥匙串上的登录、恢复、刷新、退出和历史撤销', async () => {
  const [rust, main] = await Promise.all([
    readFile(new URL('../../src-tauri/src/control_plane_auth.rs', import.meta.url), 'utf8'),
    readFile(new URL('../../src-tauri/src/main.rs', import.meta.url), 'utf8'),
  ]);

  assert.match(rust, /pub struct ControlPlaneAuthState/);
  assert.match(rust, /Mutex/);
  assert.match(rust, /state\.run_serialized/);
  assert.match(main, /manage\(ControlPlaneAuthState::default\(\)\)/);
});

test('稳定设备标识由 Rust 钥匙串持有并兼容迁移 WebView 旧值', async () => {
  const [rust, bridge, gate] = await Promise.all([
    readFile(new URL('../../src-tauri/src/control_plane_auth.rs', import.meta.url), 'utf8'),
    readFile(new URL('./controlPlaneAuth.ts', import.meta.url), 'utf8'),
    readFile(new URL('./desktop/control-plane-gate.tsx', import.meta.url), 'utf8'),
  ]);

  assert.match(rust, /DEVICE_ID_ACCOUNT/);
  assert.match(rust, /get_or_create_control_plane_device_id/);
  assert.match(bridge, /get_or_create_control_plane_device_id/);
  assert.match(gate, /await getOrCreateControlPlaneDeviceId/);
  assert.match(gate, /session\.restore\(buildDeviceRegistration\(stableDeviceId\)\)/);
});

test('退出登录完成前保持访问表单禁用，避免旧退出清理新会话', async () => {
  const gate = await readFile(new URL('./desktop/control-plane-gate.tsx', import.meta.url), 'utf8');

  assert.match(gate, /setAccessBusy\(true\)[\s\S]*await session\.logout\(\)[\s\S]*setAccessBusy\(false\)/);
});

test('设备绑定冲突和限流错误提供可执行提示并保留请求 ID', async () => {
  const [rust, session] = await Promise.all([
    readFile(new URL('../../src-tauri/src/control_plane_auth.rs', import.meta.url), 'utf8'),
    readFile(new URL('./controlPlaneSession.ts', import.meta.url), 'utf8'),
  ]);

  assert.match(rust, /request_id: Option<String>/);
  assert.match(session, /DEVICE_BINDING_CONFLICT/);
  assert.match(session, /RATE_LIMITED/);
  assert.match(session, /请求 ID/);
});
