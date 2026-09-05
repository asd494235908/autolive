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

test('管理端登录页使用映声工坊视觉底板并保留真实表单控件', async () => {
  const source = await readFile(new URL('./LoginPage.tsx', import.meta.url), 'utf8');

  assert.match(source, /assets\/yingsheng-login\.png/);
  assert.match(source, /<Form<LoginFormValues>/);
  assert.match(source, /autoComplete="username"/);
  assert.match(source, /autoComplete="current-password"/);
  assert.match(source, /htmlType="submit"/);
});

test('登录热点在 Ant Design 状态样式下仍保持参考页视觉', async () => {
  const styles = await readFile(new URL('./LoginPage.css', import.meta.url), 'utf8');

  assert.match(
    styles,
    /\.login-page \.login-stage \.login-hotspot:not\(:disabled\):hover[\s\S]*?color:\s*transparent;[\s\S]*?background:\s*transparent;/
  );
  assert.match(
    styles,
    /\.login-page \.login-stage \.login-hotspot--submit:not\(:disabled\):hover[\s\S]*?background:\s*rgb\(255 255 255 \/ 10%\);/
  );
  assert.match(
    styles,
    /\.login-page \.login-stage \.login-hotspot--submit:not\(:disabled\):active[\s\S]*?background:\s*rgb\(40 70 255 \/ 12%\);[\s\S]*?transform:\s*translateY\(0\.06cqw\);/
  );
});
