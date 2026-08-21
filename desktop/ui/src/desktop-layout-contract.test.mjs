import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

async function readSource(path) {
  return readFile(new URL(path, import.meta.url), 'utf8');
}

async function loadTypeScriptModule(path) {
  const source = await readSource(path);
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

test('主窗口保持参考图的三栏尺寸、顺序和独立滚动', async () => {
  const [app, css] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop-layout.css'),
  ]);

  assert.match(css, /\.desktop-topbar\s*\{[^}]*height:\s*36px/s);
  assert.match(css, /\.desktop-page-content\s*\{[^}]*padding:\s*12px[^}]*overflow:\s*hidden/s);
  assert.match(css, /grid-template-columns:\s*320px minmax\(0,\s*1fr\) 272px/);
  assert.match(css, /grid-template-areas:\s*"source video audio-output"/);
  assert.match(css, /gap:\s*12px/);
  assert.match(css, /\.desktop-column\s*\{[^}]*overflow:\s*hidden auto/s);
  assert.match(css, /@media\s*\(max-width:\s*1280px\)/);
  assert.match(css, /@media\s*\(max-width:\s*900px\)/);

  const source = app.indexOf('area="source"');
  const video = app.indexOf('area="video"');
  const audioOutput = app.indexOf('area="audio-output"');
  assert.ok(source >= 0 && source < video && video < audioOutput, 'DOM 顺序必须为素材→视频→声音输出');
});

test('最终效果窗口卡片按真实内容高度展示，不再固定为 96px', async () => {
  const css = await readSource('./desktop-layout.css');

  assert.match(css, /\.desktop-output-panel\s*\{[^}]*min-height:\s*120px/s);
  assert.doesNotMatch(css, /\.desktop-output-panel\s*\{[^}]*flex:\s*0\s+0\s+96px/s);
});

test('状态条和参数网格使用 Ant Design 公开组件', async () => {
  const [app, css, panel, strip, metric] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop-layout.css'),
    readSource('./desktop/desktop-panel.tsx'),
    readSource('./desktop/status-strip.tsx'),
    readSource('./desktop/parameter-metric-card.tsx'),
  ]);

  assert.match(css, /grid-template-columns:\s*repeat\(6,\s*minmax\(0,\s*1fr\)\)/);
  assert.match(css, /grid-template-columns:\s*repeat\(7,\s*minmax\(0,\s*1fr\)\)/);
  assert.equal((app.match(/key:\s*'(?:playback|loop|video|audio|source|output)'/g) ?? []).length, 6);
  assert.equal((app.match(/label:\s*'(?:亮度|对比度|饱和度|色相旋转|模糊|锐化|噪点|细节增强|缩放|动态裁剪|水平偏移|垂直偏移)'/g) ?? []).length, 12);
  assert.match(panel, /import \{ Card \} from 'antd'/);
  assert.match(strip, /import \{ Badge, Button, Card, Progress \} from 'antd'/);
  assert.match(metric, /import \{ Badge, Card, Progress, Slider \} from 'antd'/);
  assert.match(app, /普通声音 · 参数状态/);
  assert.match(app, /声音预设池 · 当前选择/);
  assert.doesNotMatch(app, /addonBefore|addonAfter/);
  assert.doesNotMatch(css, /\.ant-/);
});

test('参数卡只在真实值或实际预设参与集合变化时生成新闪动 token', async () => {
  const {
    advanceMetricFlashTokens,
    getChangedMetricKeys,
    getNewlyActivePresetIds,
  } = await loadTypeScriptModule('./desktop/metric-change-flash.ts');
  const first = { brightness: 0, contrast: 100, gain: 0 };

  assert.deepEqual(getChangedMetricKeys(null, first), []);
  assert.deepEqual(getChangedMetricKeys(first, { ...first }), []);
  assert.deepEqual(
    getChangedMetricKeys(first, { brightness: 0.25, contrast: 100, gain: -1 }),
    ['brightness', 'gain'],
  );

  const initialTokens = {};
  assert.equal(advanceMetricFlashTokens(initialTokens, []), initialTokens);
  const changedTokens = advanceMetricFlashTokens(initialTokens, ['brightness', 'gain']);
  assert.deepEqual(changedTokens, { brightness: 1, gain: 1 });
  assert.deepEqual(advanceMetricFlashTokens(changedTokens, ['brightness']), { brightness: 2, gain: 1 });

  assert.deepEqual(getNewlyActivePresetIds(null, ['p1']), []);
  assert.deepEqual(getNewlyActivePresetIds(['p1', 'p2'], ['p2', 'p1']), []);
  assert.deepEqual(getNewlyActivePresetIds(['p1', 'p2'], ['p2']), []);
  assert.deepEqual(getNewlyActivePresetIds(['p1'], ['p1', 'p3', 'p3']), ['p3']);
});

test('参数卡闪动轻量、单次且尊重减少动态效果偏好', async () => {
  const [app, card, css] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop/parameter-metric-card.tsx'),
    readSource('./desktop-layout.css'),
  ]);

  assert.match(app, /audio_processing_runtime/);
  assert.match(app, /setActualAudioPresetIds\(plan\.sample\.sample\.presetIds\)/);
  assert.match(app, /flashToken=\{metricFlashTokens\[`video:\$\{metric\.field\}`\]\}/);
  assert.match(app, /flashToken=\{metricFlashTokens\[`audio:\$\{row\.key\}`\]\}/);
  assert.match(app, /flashToken=\{metricFlashTokens\[`preset:\$\{preset\.id\}`\]\}/);
  assert.match(card, /parameter-metric-card-flash-\$\{flashToken % 2\}/);
  assert.match(css, /parameter-metric-card-flash-0[^}]*220ms ease-out/);
  assert.match(css, /parameter-metric-card-flash-1[^}]*220ms ease-out/);
  assert.match(css, /@media\s*\(prefers-reduced-motion:\s*reduce\)/);
  assert.match(css, /parameter-metric-card-flash-0,\s*\.parameter-metric-card-flash-1\s*\{\s*animation:\s*none/);
  assert.doesNotMatch(css, /\.ant-/);
});

test('主窗口只暴露本期视频参数，并保留插话与固定话术抽屉', async () => {
  const app = await readSource('./App.tsx');
  const desktop = app.slice(app.indexOf('function DesktopApp()'));

  assert.match(desktop, /title="随机插话"/);
  assert.match(desktop, /title="固定话术"/);
  assert.match(desktop, /title="高级声音设置"/);
  assert.doesNotMatch(desktop, /实时话术幻化/);
  assert.doesNotMatch(desktop, /研究 Worker|研究 MP4|内容指纹|65Hz|20000Hz/);
});

test('三个功能抽屉复用响应式高密度结构并保留清晰操作区', async () => {
  const [app, css, drawer] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop-layout.css'),
    readSource('./desktop/feature-drawer.tsx'),
  ]);

  assert.match(drawer, /import \{ Card, Drawer, Typography \} from 'antd'/);
  assert.match(drawer, /width=\{`min\(\$\{width\}px, 100vw\)`\}/);
  assert.match(drawer, /footer=\{footer \? <div className="feature-drawer-footer">/);
  assert.match(drawer, /aria-live="polite"/);
  assert.match(drawer, /mask: \{ background: 'rgba\(0, 0, 0, 0\.64\)' \}/);
  assert.equal((app.match(/<FeatureDrawer\b/g) ?? []).length, 3);
  assert.match(app, /title="高级声音设置"[\s\S]*width=\{760\}/);
  assert.match(app, /title="随机插话"[\s\S]*width=\{560\}/);
  assert.match(app, /title="固定话术"[\s\S]*width=\{560\}/);
  for (const label of ['插话音频目录', '预制标题', '朗读正文', 'Host API', '输出设备', '内存缓冲', '声音参数值预设']) {
    assert.match(app, new RegExp(`label="${label}"`), `抽屉字段缺少可见标签：${label}`);
  }
  assert.match(app, /删除这条预制文本？/);
  assert.match(app, /ariaLabel="原声压低过渡" label="原声压低过渡"/);
  assert.match(app, /ariaLabel="原声恢复过渡" label="原声恢复过渡"/);
  assert.match(app, /重新生成本周期参数/);
  assert.match(app, /应用声音参数/);
  assert.match(css, /feature-drawer-checkbox-grid[^}]*repeat\(4,/);
  assert.match(css, /@media\s*\(max-width:\s*900px\)[\s\S]*feature-drawer-checkbox-grid[^}]*repeat\(2,/);
  assert.match(css, /@media\s*\(max-width:\s*600px\)[\s\S]*feature-drawer-field-grid[\s\S]*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  assert.doesNotMatch(css, /\.ant-/);
});

test('顶栏使用真实图标资源和安全的 Tauri 窗口动作', async () => {
  const [shell, capabilitySource, tauriConfigSource] = await Promise.all([
    readSource('./desktop/desktop-shell.tsx'),
    readSource('../../src-tauri/capabilities/default.json'),
    readSource('../../src-tauri/tauri.conf.json'),
  ]);
  const capability = JSON.parse(capabilitySource);
  const tauriConfig = JSON.parse(tauriConfigSource);

  assert.match(shell, /src="\/app-icon\.png"/);
  assert.ok((shell.match(/data-tauri-drag-region/g) ?? []).length >= 5);
  assert.match(shell, /'__TAURI_INTERNALS__' in window/);
  for (const label of ['最小化窗口', '最大化或还原窗口', '关闭窗口']) {
    assert.match(shell, new RegExp(`aria-label="${label}"`));
  }
  assert.ok(capability.windows.includes('main'));
  assert.ok(tauriConfig.app.security.capabilities.includes(capability.identifier));
  for (const permission of [
    'core:window:allow-minimize',
    'core:window:allow-toggle-maximize',
    'core:window:allow-close',
    'core:window:allow-start-dragging',
  ]) {
    assert.ok(capability.permissions.includes(permission), `缺少窗口权限：${permission}`);
  }
});
