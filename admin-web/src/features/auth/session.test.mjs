import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./session.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 }
}).outputText;

const values = new Map();
globalThis.sessionStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, value),
  removeItem: (key) => values.delete(key),
  clear: () => values.clear(),
  key: () => null,
  length: 0
};

const sessionModule = await import(
  `data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`
);

const session = {
  tokens: {
    access_token: 'access-old',
    refresh_token: 'refresh-old',
    expires_at: '2026-08-13T00:00:00Z'
  },
  user: { id: 'usr_1' }
};

test('refresh token rotation updates only the stored session tokens', () => {
  sessionModule.saveSession(session);
  sessionModule.updateSessionTokens({
    access_token: 'access-new',
    refresh_token: 'refresh-new',
    expires_at: '2026-08-13T01:00:00Z'
  });

  assert.deepEqual(sessionModule.readSession(), {
    ...session,
    tokens: {
      access_token: 'access-new',
      refresh_token: 'refresh-new',
      expires_at: '2026-08-13T01:00:00Z'
    }
  });
});
