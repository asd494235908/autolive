import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

async function loadPolicy() {
  const source = await readFile(new URL('./controlPlaneGatePolicy.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

const NOW = Date.parse('2026-08-21T10:00:00.000Z');
const activeUser = { id: 'user-current', status: 'active' };
const activeDevice = {
  id: 'desktop-current',
  user_id: 'user-current',
  status: 'active',
  activation_expires_at: '2026-08-21T10:01:00.000Z',
};

test('只有当前设备、启用用户和未过期激活状态同时成立才允许进入工作台', async () => {
  const { isConfirmedDesktopAccess } = await loadPolicy();

  assert.equal(isConfirmedDesktopAccess(activeUser, activeDevice, 'desktop-current', NOW), true);
  assert.equal(isConfirmedDesktopAccess(activeUser, activeDevice, '', NOW), false);
  assert.equal(isConfirmedDesktopAccess({ status: 'disabled' }, activeDevice, 'desktop-current', NOW), false);
  assert.equal(isConfirmedDesktopAccess(activeUser, { ...activeDevice, user_id: 'user-other' }, 'desktop-current', NOW), false);
  assert.equal(isConfirmedDesktopAccess(activeUser, { ...activeDevice, id: 'desktop-other' }, 'desktop-current', NOW), false);
  assert.equal(isConfirmedDesktopAccess(activeUser, { ...activeDevice, status: 'disabled' }, 'desktop-current', NOW), false);
  assert.equal(isConfirmedDesktopAccess(activeUser, { ...activeDevice, status: 'revoked' }, 'desktop-current', NOW), false);
  assert.equal(isConfirmedDesktopAccess(activeUser, { ...activeDevice, status: 'pending_activation' }, 'desktop-current', NOW), false);
});

test('激活到期时间必须可解析且严格晚于当前时间', async () => {
  const { isConfirmedDesktopAccess } = await loadPolicy();

  assert.equal(isConfirmedDesktopAccess(activeUser, { ...activeDevice, activation_expires_at: '2026-08-21T10:00:00.000Z' }, 'desktop-current', NOW), false);
  assert.equal(isConfirmedDesktopAccess(activeUser, { ...activeDevice, activation_expires_at: '2026-08-21T09:59:59.999Z' }, 'desktop-current', NOW), false);
  assert.equal(isConfirmedDesktopAccess(activeUser, { ...activeDevice, activation_expires_at: 'not-a-date' }, 'desktop-current', NOW), false);
  assert.equal(isConfirmedDesktopAccess(activeUser, { ...activeDevice, activation_expires_at: null }, 'desktop-current', NOW), false);
});

test('同账号同设备的待授权或过期绑定可自动重绑，禁用和错配设备不可重绑', async () => {
  const { shouldAutomaticallyRegisterDevice } = await loadPolicy();

  assert.equal(shouldAutomaticallyRegisterDevice(
    { user: activeUser, device: { ...activeDevice, status: 'pending_activation' } },
    'desktop-current',
    NOW,
  ), true);
  assert.equal(shouldAutomaticallyRegisterDevice(
    { user: activeUser, device: { ...activeDevice, activation_expires_at: '2026-08-21T10:00:00.000Z' } },
    'desktop-current',
    NOW,
  ), true);
  assert.equal(shouldAutomaticallyRegisterDevice(
    { user: activeUser, device: { ...activeDevice, status: 'disabled' } },
    'desktop-current',
    NOW,
  ), false);
  assert.equal(shouldAutomaticallyRegisterDevice(
    { user: { ...activeUser, status: 'disabled' }, device: { ...activeDevice, status: 'pending_activation' } },
    'desktop-current',
    NOW,
  ), false);
  assert.equal(shouldAutomaticallyRegisterDevice(
    { user: activeUser, device: { ...activeDevice, user_id: 'user-other', status: 'pending_activation' } },
    'desktop-current',
    NOW,
  ), false);
  assert.equal(shouldAutomaticallyRegisterDevice(
    { user: activeUser, device: { ...activeDevice, id: 'desktop-other', status: 'pending_activation' } },
    'desktop-current',
    NOW,
  ), false);
});
