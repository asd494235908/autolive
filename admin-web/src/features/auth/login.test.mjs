import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('管理端登录提交 AutoLive 产品标识', async () => {
  const source = await readFile(new URL('./LoginPage.tsx', import.meta.url), 'utf8');

  assert.match(
    source,
    /body:\s*\{\s*\.\.\.values,\s*product:\s*'autolive'\s*\}/s
  );
});
