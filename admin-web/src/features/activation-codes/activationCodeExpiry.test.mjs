import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./activationCodeExpiry.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 }
}).outputText;
const expiryModule = await import(
  `data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`
);
const pageSource = await readFile(new URL('./ActivationCodesPage.tsx', import.meta.url), 'utf8');

test('defines the requested activation code expiry presets', () => {
  assert.deepEqual(
    expiryModule.ACTIVATION_CODE_EXPIRY_PRESETS,
    [
      { label: '3天', amount: 3, unit: 'day' },
      { label: '7天', amount: 7, unit: 'day' },
      { label: '30天', amount: 30, unit: 'day' },
      { label: '90天', amount: 90, unit: 'day' },
      { label: '1年', amount: 1, unit: 'year' }
    ]
  );
});

test('adds a day-based expiry while preserving the selected time', () => {
  const now = new Date('2026-01-01T10:20:30.000Z');

  assert.equal(
    expiryModule.calculateActivationCodeExpiry(now, 30, 'day').toISOString(),
    '2026-01-31T10:20:30.000Z'
  );
});

test('adds a calendar year for the one-year preset', () => {
  const now = new Date('2026-01-01T10:20:30.000Z');

  assert.equal(
    expiryModule.calculateActivationCodeExpiry(now, 1, 'year').toISOString(),
    '2027-01-01T10:20:30.000Z'
  );
});

test('uses an Ant Design DatePicker and serializes its value at the request boundary', () => {
  assert.match(pageSource, /DatePicker/);
  assert.match(pageSource, /showTime/);
  assert.match(pageSource, /presets/);
  assert.match(pageSource, /toISOString\(\)/);
  assert.doesNotMatch(pageSource, /\bInput\b/);
  assert.doesNotMatch(pageSource, /请输入 RFC3339 时间/);
});

test('allows choosing activation capacity and shows binding progress', () => {
  assert.match(pageSource, /InputNumber/);
  assert.match(pageSource, /max_devices:\s*values\.max_devices/);
  assert.match(pageSource, /bound_devices/);
  assert.match(pageSource, /\['active', 'used'\]\.includes\(record\.status\)/);
});

test('requires choosing an active account in the current product before creation', () => {
  assert.match(pageSource, /apiClient\.get<UserListResponse>\('\/api\/v1\/admin\/users'/);
  assert.match(pageSource, /page_size:\s*200/);
  assert.match(pageSource, /product:\s*authorization\.product/);
  assert.match(pageSource, /authorization\.can\('users\.read'\)/);
  assert.match(pageSource, /disabled=\{!canCreateActivationCodes\}/);
  assert.match(pageSource, /缺少账号读取权限/);
  assert.match(pageSource, /绑定账号加载失败/);
  assert.match(pageSource, /user\.status === 'active'/);
  assert.match(pageSource, /<Select/);
  assert.match(pageSource, /showSearch/);
  assert.match(pageSource, /optionFilterProp="label"/);
  assert.match(pageSource, /user_id:\s*values\.user_id/);
  assert.match(pageSource, /dataIndex:\s*'user_id'/);
  assert.match(pageSource, /最多登录设备数/);
});
