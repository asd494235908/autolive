import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const require = createRequire(import.meta.url);

async function loadStartupModule() {
  const source = await readFile(new URL('./启动加载.tsx', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: {
      jsx: ts.JsxEmit.ReactJSX,
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2021,
    },
  }).outputText;
  const module = { exports: {} };
  new Function('exports', 'require', 'module', compiled)(module.exports, require, module);
  return module.exports;
}

function readText(node) {
  if (typeof node === 'string' || typeof node === 'number') return String(node);
  if (!node || !node.props) return '';
  return [node.props.children]
    .flat(Infinity)
    .map(readText)
    .join('');
}

function findElement(node, predicate) {
  if (!node || !node.props) return null;
  if (predicate(node)) return node;
  for (const child of [node.props.children].flat(Infinity)) {
    const matched = findElement(child, predicate);
    if (matched) return matched;
  }
  return null;
}

test('启动 Loading 显示可访问的进行状态和指定提示', async () => {
  const { StartupLoading } = await loadStartupModule();
  const view = StartupLoading({ message: '正在加载主界面…' });

  assert.equal(view.props.role, 'status');
  assert.equal(view.props['aria-live'], 'polite');
  assert.match(readText(view), /正在加载主界面/);
});

test('顶层错误边界显示错误并允许重新加载', async () => {
  const { StartupErrorBoundary } = await loadStartupModule();
  const error = new Error('主界面模块加载失败');
  const boundary = new StartupErrorBoundary({ children: '应用内容' });
  const nextState = StartupErrorBoundary.getDerivedStateFromError(error);

  assert.equal(nextState.error, error);
  boundary.state = nextState;
  const view = boundary.render();
  assert.equal(view.props.role, 'alert');
  assert.match(readText(view), /主界面模块加载失败/);
  const reloadButton = findElement(view, (element) => element.type === 'button' && readText(element) === '重新加载');
  assert.ok(reloadButton);

  const previousWindow = globalThis.window;
  let reloadCount = 0;
  globalThis.window = { location: { reload: () => { reloadCount += 1; } } };
  try {
    reloadButton.props.onClick();
    assert.equal(reloadCount, 1);
  } finally {
    if (previousWindow) globalThis.window = previousWindow;
    else delete globalThis.window;
  }
});
