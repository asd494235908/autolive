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
});
