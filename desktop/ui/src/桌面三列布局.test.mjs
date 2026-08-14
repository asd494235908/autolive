import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const appPath = new URL('./App.tsx', import.meta.url);
const layoutPath = new URL('./desktop-layout.css', import.meta.url);

test('desktop page exposes the approved three-column layout contract', async () => {
  const app = await readFile(appPath, 'utf8');
  let css = '';
  try {
    css = await readFile(layoutPath, 'utf8');
  } catch {
    // Keep the failure an assertion failure until the layout stylesheet exists.
  }

  assert.match(app, /desktop-workspace/);
  assert.match(app, /desktop-column-source/);
  assert.match(app, /desktop-column-audio/);
  assert.match(app, /desktop-column-video/);
  assert.match(css, /grid-template-columns:\s*minmax\(0,\s*28fr\)\s+minmax\(0,\s*38fr\)\s+minmax\(0,\s*34fr\)/);
  assert.match(css, /@media\s*\(max-width:\s*800px\)/);
  assert.match(css, /grid-template-columns:\s*1fr/);
});
