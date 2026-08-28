import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

async function loadPreferences() {
  const source = await readFile(new URL('./authFormPreferences.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

async function loadMemoryWithMocks() {
  globalThis.__authFormMemoryTest = {
    operations: [],
    preferences: { username: '', rememberLogin: false },
  };
  const authMock = `
    export async function clearLegacyAuthCredentials(deviceId) {
      globalThis.__authFormMemoryTest.operations.push(['clear-legacy', deviceId]);
    }
  `;
  const preferenceMock = `
    export function loadAuthFormPreferences() {
      return { ...globalThis.__authFormMemoryTest.preferences };
    }
    export function saveAuthFormPreferences(deviceId, preferences) {
      globalThis.__authFormMemoryTest.operations.push(['save-preferences', deviceId, { ...preferences }]);
      globalThis.__authFormMemoryTest.preferences = { ...preferences };
    }
  `;
  const authUrl = `data:text/javascript;charset=utf-8,${encodeURIComponent(authMock)}`;
  const preferenceUrl = `data:text/javascript;charset=utf-8,${encodeURIComponent(preferenceMock)}`;
  const source = await readFile(new URL('./authFormMemory.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText
    .replace("'./controlPlaneAuth'", JSON.stringify(authUrl))
    .replace("'./authFormPreferences'", JSON.stringify(preferenceUrl));
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}#${Date.now()}-${Math.random()}`);
}

function createStorage() {
  const values = new Map();
  return {
    getItem(key) { return values.get(key) ?? null; },
    setItem(key, value) { values.set(key, String(value)); },
    removeItem(key) { values.delete(key); },
    dump() { return [...values.entries()]; },
  };
}

test('账号偏好按设备隔离且普通存储中不包含密码或激活码字段', async () => {
  const { loadAuthFormPreferences, saveAuthFormPreferences } = await loadPreferences();
  const storage = createStorage();

  saveAuthFormPreferences('desktop-a', {
    username: 'alice',
    rememberLogin: true,
  }, storage);

  assert.deepEqual(loadAuthFormPreferences('desktop-a', storage), {
    username: 'alice',
    rememberLogin: true,
  });
  assert.deepEqual(loadAuthFormPreferences('desktop-b', storage), {
    username: '',
    rememberLogin: false,
  });
  const serialized = JSON.stringify(storage.dump());
  assert.doesNotMatch(serialized, /"password"|"activationCode"|"activation_code"|"secret"/i);
});

test('损坏的账号偏好显式报错，不以本地布尔值放行', async () => {
  const { loadAuthFormPreferences } = await loadPreferences();
  const storage = createStorage();
  storage.setItem('autolive.desktop.auth-form.desktop-a.v1', '{bad json');

  assert.throws(() => loadAuthFormPreferences('desktop-a', storage), /账号偏好读取失败/);
});

test('只记忆账号偏好并由 Rust 幂等清理历史密码和激活码凭据', async () => {
  const memory = await loadMemoryWithMocks();

  await memory.saveRememberedLogin('desktop-a', 'alice');
  await memory.clearLegacyRememberedAuthCredentials('desktop-a');
  await memory.clearRememberedLogin('desktop-a');

  const operations = globalThis.__authFormMemoryTest.operations;
  assert.ok(operations.some((operation) => operation[0] === 'clear-legacy'));
  const preferenceWrites = operations.filter((operation) => operation[0] === 'save-preferences');
  assert.equal(JSON.stringify(preferenceWrites).includes('password'), false);
  assert.equal(JSON.stringify(preferenceWrites).includes('rememberActivationCode'), false);
});

test('认证 IPC 不暴露 Refresh Token 或密码存取命令', async () => {
  const source = await readFile(new URL('./controlPlaneAuth.ts', import.meta.url), 'utf8');

  assert.match(source, /login_control_plane_session/);
  assert.match(source, /restore_control_plane_session/);
  assert.match(source, /refresh_control_plane_session/);
  assert.match(source, /logout_control_plane_session/);
  assert.match(source, /clear_legacy_auth_credentials/);
  assert.doesNotMatch(source, /refreshToken|refresh_token|storeLoginPassword|loadLoginPassword/);
  assert.doesNotMatch(source, /localStorage|sessionStorage/);
});

test('桌面表单只提交账号密码，自动填充不自动登录', async () => {
  const [gate, panel] = await Promise.all([
    readFile(new URL('./desktop/control-plane-gate.tsx', import.meta.url), 'utf8'),
    readFile(new URL('./desktop/control-plane-access-panel.tsx', import.meta.url), 'utf8'),
  ]);
  const memory = await readFile(new URL('./authFormMemory.ts', import.meta.url), 'utf8');

  assert.match(gate, /<ControlPlaneAccessPanel/);
  assert.doesNotMatch(gate, /function LoginPanel|function ActivationPanel/);
  assert.match(panel, /name="username"/);
  assert.match(panel, /name="password"/);
  assert.match(panel, /记住账号/);
  assert.match(panel, /form\.setFieldsValue/);
  assert.match(panel, /正在读取已保存的信息/);
  assert.match(panel, /clearRememberedLogin/);
  assert.match(panel, /清除已记住账号/);
  assert.doesNotMatch(panel, /activationCode|激活码/);
  assert.doesNotMatch(`${gate}\n${panel}`, /localStorage|sessionStorage/);
  assert.doesNotMatch(memory, /password|login_password/);
  assert.doesNotMatch(memory, /saveRememberedActivationCode|loadRememberedActivationCode/);
  assert.doesNotMatch(memory, /localStorage|sessionStorage/);
  assert.doesNotMatch(panel, /setFieldsValue[\s\S]{0,120}(submit|onSubmit)\(/);
});

test('用户操作只提交一次账号密码，设备注册由会话层自动完成', async () => {
  const [gate, panel] = await Promise.all([
    readFile(new URL('./desktop/control-plane-gate.tsx', import.meta.url), 'utf8'),
    readFile(new URL('./desktop/control-plane-access-panel.tsx', import.meta.url), 'utf8'),
  ]);

  assert.match(gate, /await session\.login\(/);
  assert.equal((gate.match(/await session\.login\(/g) ?? []).length, 1);
  assert.doesNotMatch(gate, /session\.activate|activationCode/);
  assert.match(gate, /status:\s*result\.status/);
  assert.match(panel, /aria-live="polite"/);
});

test('凭据成功提示自动消失且新操作、卸载和竞态不会误清警告', async () => {
  const [gate, panel] = await Promise.all([
    readFile(new URL('./desktop/control-plane-gate.tsx', import.meta.url), 'utf8'),
    readFile(new URL('./desktop/control-plane-access-panel.tsx', import.meta.url), 'utf8'),
  ]);

  assert.match(gate, /const CREDENTIAL_SUCCESS_NOTICE_DURATION_MS = 4_000/);
  assert.match(gate, /if \(notice\?\.type !== 'success'\) return/);
  assert.match(gate, /window\.setTimeout\([\s\S]*setNotice\(\(current\) => current === notice \? null : current\)/);
  assert.match(gate, /return \(\) => window\.clearTimeout\(timer\)/);
  assert.equal((gate.match(/= useCredentialNotice\(\)/g) ?? []).length, 1);
  assert.ok((gate.match(/setCredentialNotice\(null\)/g) ?? []).length >= 2);
  assert.ok((panel.match(/onCredentialNotice\(null\)/g) ?? []).length >= 2);
});
