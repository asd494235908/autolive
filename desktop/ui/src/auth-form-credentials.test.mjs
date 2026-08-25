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
    credentials: { login_password: 'remembered-password', activation_code: 'remembered-code' },
    operations: [],
    preferences: { username: '', rememberLogin: false },
  };
  const credentialMock = `
    export async function storeLoginPassword(deviceId, value) {
      globalThis.__authFormMemoryTest.operations.push(['store', deviceId, 'login_password', value]);
      globalThis.__authFormMemoryTest.credentials.login_password = value;
    }
    export async function loadLoginPassword(deviceId) {
      globalThis.__authFormMemoryTest.operations.push(['load', deviceId, 'login_password']);
      return globalThis.__authFormMemoryTest.credentials.login_password ?? null;
    }
    export async function deleteLoginPassword(deviceId) {
      globalThis.__authFormMemoryTest.operations.push(['delete', deviceId, 'login_password']);
      delete globalThis.__authFormMemoryTest.credentials.login_password;
    }
    export async function deleteLegacyActivationCode(deviceId) {
      globalThis.__authFormMemoryTest.operations.push(['delete', deviceId, 'activation_code']);
      delete globalThis.__authFormMemoryTest.credentials.activation_code;
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
  const credentialUrl = `data:text/javascript;charset=utf-8,${encodeURIComponent(credentialMock)}`;
  const preferenceUrl = `data:text/javascript;charset=utf-8,${encodeURIComponent(preferenceMock)}`;
  const source = await readFile(new URL('./authFormMemory.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText
    .replace("'./authCredentialStore'", JSON.stringify(credentialUrl))
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

test('只写入账号密码并幂等清理历史激活码凭据', async () => {
  const memory = await loadMemoryWithMocks();

  await memory.saveRememberedLogin('desktop-a', 'alice', 'new-password');
  await memory.clearLegacyRememberedActivationCode('desktop-a');
  await memory.clearRememberedLogin('desktop-a');

  const operations = globalThis.__authFormMemoryTest.operations;
  assert.deepEqual(operations[0], ['store', 'desktop-a', 'login_password', 'new-password']);
  assert.ok(operations.some((operation) => operation[0] === 'delete' && operation[2] === 'login_password'));
  assert.ok(operations.some((operation) => operation[0] === 'delete' && operation[2] === 'activation_code'));
  const preferenceWrites = operations.filter((operation) => operation[0] === 'save-preferences');
  assert.equal(JSON.stringify(preferenceWrites).includes('new-password'), false);
  assert.equal(JSON.stringify(preferenceWrites).includes('rememberActivationCode'), false);
});

test('密码只通过 Tauri 安全凭据 IPC，激活码仅保留历史删除入口', async () => {
  const source = await readFile(new URL('./authCredentialStore.ts', import.meta.url), 'utf8');

  assert.match(source, /storeLoginPassword/);
  assert.match(source, /loadLoginPassword/);
  assert.match(source, /deleteLoginPassword/);
  assert.match(source, /deleteLegacyActivationCode/);
  assert.doesNotMatch(source, /storeAuthFormCredential|loadAuthFormCredential/);
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
  assert.match(panel, /记住账号和密码/);
  assert.match(panel, /form\.setFieldsValue/);
  assert.match(panel, /正在读取已保存的信息/);
  assert.match(panel, /clearRememberedLogin/);
  assert.match(panel, /清除账号密码/);
  assert.doesNotMatch(panel, /activationCode|激活码/);
  assert.doesNotMatch(`${gate}\n${panel}`, /localStorage|sessionStorage/);
  assert.match(memory, /storeLoginPassword\(deviceId, password\)/);
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
