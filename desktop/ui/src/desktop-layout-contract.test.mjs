import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

async function readSource(path) {
  return readFile(new URL(path, import.meta.url), 'utf8');
}

async function loadTypeScriptModule(path) {
  const source = (await readSource(path)).replace(
    "import { AUDIO_MIX_PICK_HARD_MAX } from './runtime-parameter-scheduler';",
    'const AUDIO_MIX_PICK_HARD_MAX = 4;',
  );
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

  assert.match(css, /\.desktop-topbar\s*\{[^}]*height:\s*var\(--desktop-titlebar-height\)/s);
  assert.match(css, /--desktop-titlebar-height:\s*36px/);
  assert.match(css, /\.desktop-window-frame\s*\{[^}]*grid-template-rows:\s*var\(--desktop-titlebar-height\) minmax\(0,\s*1fr\)/s);
  assert.match(css, /\.desktop-window-client\s*\{[^}]*overflow:\s*hidden/s);
  assert.match(css, /\.desktop-page\s*\{[^}]*width:\s*100%/s);
  assert.doesNotMatch(css, /\.desktop-page\s*\{[^}]*width:\s*100vw/s);
  assert.match(css, /\.desktop-page-content\s*\{[^}]*padding:\s*12px[^}]*overflow:\s*hidden/s);
  assert.match(css, /grid-template-columns:\s*320px minmax\(0,\s*1fr\) 272px/);
  assert.match(css, /grid-template-areas:\s*"source media output"/);
  assert.match(css, /gap:\s*12px/);
  assert.match(css, /\.desktop-panel\s*\{[^}]*border-radius:\s*10px/s);
  assert.match(css, /\.desktop-source-pool\s*\{[^}]*min-height:\s*328px[^}]*flex:\s*0\s+0\s+328px/s);
  assert.match(css, /\.desktop-source-current\s*>\s*\.desktop-source-list\s*\{[^}]*max-height:\s*214px/s);
  assert.match(css, /\.desktop-column\s*\{[^}]*overflow:\s*hidden auto/s);
  assert.match(css, /@media\s*\(max-width:\s*1199px\)[\s\S]*?\.desktop-page-content\s*\{[^}]*overflow:\s*auto/s);
  assert.doesNotMatch(css, /@media\s*\(max-width:\s*1199px\)[\s\S]*?\.desktop-page\s*\{[^}]*overflow:\s*auto/s);
  assert.match(css, /@media\s*\(max-width:\s*1199px\)[\s\S]*?\.desktop-workspace\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  assert.match(css, /@media\s*\(max-width:\s*900px\)/);
  assert.match(css, /@media\s*\(max-width:\s*720px\)[\s\S]*?\.desktop-media-domain-grid,\s*\.desktop-column-media\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/s);
  assert.match(css, /@media\s*\(max-width:\s*720px\)[\s\S]*?\.desktop-auth-session-controls\s*\{[^}]*right:\s*8px/s);
  assert.match(css, /\.desktop-auth-session-expiry\s*\{[^}]*text-overflow:\s*ellipsis/s);
  assert.match(css, /@media\s*\(max-width:\s*480px\)[\s\S]*?\.desktop-page-content\s*\{[^}]*padding:\s*8px/s);
  assert.match(css, /@media\s*\(prefers-reduced-motion:\s*reduce\)/);

  const source = app.indexOf('area="source"');
  const media = app.indexOf('area="media"');
  const output = app.indexOf('area="output"');
  assert.ok(source >= 0 && source < media && media < output, 'DOM 顺序必须为播放区→音视频双卡→输出状态');
  assert.match(css, /\.desktop-column-media\s*\{[^}]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/s);
});

test('最终效果窗口卡片按设计稿紧凑高度展示', async () => {
  const css = await readSource('./desktop-layout.css');

  assert.match(css, /\.desktop-output-panel\s*\{[^}]*min-height:\s*106px/s);
  assert.doesNotMatch(css, /\.desktop-output-panel\s*\{[^}]*flex:\s*0\s+0\s+96px/s);
});

test('普通声音面板按内容自然增高并由右列负责滚动', async () => {
  const css = await readSource('./desktop-layout.css');
  const audioPanelRule = css.match(/\.desktop-audio-panel\s*\{([^}]*)\}/s)?.[1] ?? '';

  assert.match(css, /\.desktop-column\s*\{[^}]*overflow:\s*hidden auto/s);
  assert.match(audioPanelRule, /height:\s*auto/);
  assert.doesNotMatch(audioPanelRule, /overflow:\s*visible/);
  assert.doesNotMatch(audioPanelRule, /(?:height|min-height):\s*544px/);
  assert.doesNotMatch(audioPanelRule, /flex:\s*0\s+0\s+544px/);
  assert.match(
    css,
    /@media\s*\(max-width:\s*1199px\)[\s\S]*?\.desktop-column\s*\{[^}]*overflow:\s*visible/s,
  );
});

test('主页周期卡和正式参数面板使用 Ant Design 公开组件', async () => {
  const [app, css, panel, cycleCard, parameterPanels] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop-layout.css'),
    readSource('./desktop/desktop-panel.tsx'),
    readSource('./desktop/media-cycle-card.tsx'),
    readSource('./media-parameter-panels/MediaParameterPanels.tsx'),
  ]);

  assert.match(panel, /import \{ Card \} from 'antd'/);
  assert.match(cycleCard, /import \{[^}]*\bProgress\b[^}]*\} from 'antd'/);
  assert.match(cycleCard, /<Progress aria-label=\{`\$\{title\}下一计划进度`\}/);
  assert.match(cycleCard, /desktop-cycle-meta[\s\S]*?<CompactNumberField ariaLabel=\{`\$\{title\}最小秒`\}/);
  assert.match(cycleCard, /ariaLabel=\{`\$\{title\}(?:最小|最大)秒`\}[^>]*size="small"/);
  assert.match(cycleCard, /最小秒[^\n]*max=\{range\.maxMs \/ 1000\}[^\n]*onRangeChange\?\.\('min', value \* 1000\)/);
  assert.match(cycleCard, /最大秒[^\n]*min=\{range\.minMs \/ 1000\}[^\n]*onRangeChange\?\.\('max', value \* 1000\)/);
  assert.match(app, /setAudioPeriodRange\(\(current\) => updatePeriodRangeEndpoint\(current, endpoint, valueMs\)\)/);
  assert.match(app, /setVideoPeriodRange\(\(current\) => updatePeriodRangeEndpoint\(current, endpoint, valueMs\)\)/);
  assert.doesNotMatch(cycleCard, /desktop-cycle-inputs/);
  assert.match(panel, /titleIcon\?: ReactNode/);
  assert.match(panel, /subtitle\?: ReactNode/);
  assert.match(parameterPanels, /from 'antd'/);
  assert.doesNotMatch(parameterPanels, /\bProgress\b/);
  assert.match(app, /<MediaParameterPanels/);
  assert.match(app, /const AUDIO_PARAMETER_SECTIONS = \['audio'\] as const/);
  assert.match(app, /const VIDEO_PARAMETER_SECTIONS = \['video', 'advanced'\] as const/);
  assert.match(app, /sections=\{AUDIO_PARAMETER_SECTIONS\}/);
  assert.match(app, /sections=\{VIDEO_PARAMETER_SECTIONS\}/);
  assert.doesNotMatch(app, /addonBefore|addonAfter/);
  assert.doesNotMatch(css, /\.ant-/);
});

test('主页声音参数保持只读，高级抽屉承载可编辑声音控件', async () => {
  const app = await readSource('./App.tsx');
  const home = app.slice(
    app.indexOf('<DesktopShell>'),
    app.indexOf('title="随机插话"'),
  );
  const advancedDrawer = app.slice(
    app.indexOf('title="高级声音设置"'),
    app.indexOf('title={`当前声音预设'),
  );

  assert.match(app, /const audioParameterControls = mediaEffectParams \? \(/);
  assert.match(app, /<AudioParameterControls/);
  assert.match(app, /onChange=\{\(field, value\) => updateMediaEffectParam\('audio', field, value\)\}/);
  assert.doesNotMatch(home, /audioControls=\{audioParameterControls\}/);
  assert.match(advancedDrawer, /\{audioParameterControls\}/);
  assert.doesNotMatch(advancedDrawer, /title="音高、变速与共振峰"/);
  assert.doesNotMatch(advancedDrawer, /title="特征、信噪比与空间混合"/);
});

test('声音预设详情完整展示 35 个效果字段并格式化非数值类型', async () => {
  const app = await readSource('./App.tsx');
  const definitions = app.slice(
    app.indexOf('const AUDIO_PRESET_FIELD_DEFINITIONS'),
    app.indexOf('function formatAudioPresetFieldValue'),
  );
  const formatter = app.slice(
    app.indexOf('function formatAudioPresetFieldValue'),
    app.indexOf('function getPlaybackDisplayLabel'),
  );
  const expectedFields = [
    'natural_voice_mode', 'pitch_shift_semitones', 'spectral_perturbation_percent',
    'environment_noise_percent', 'environment_noise_dbfs', 'mfcc_shift_percent',
    'phase_perturbation_percent', 'loudness_adjustment_db', 'input_gain_db',
    'output_gain_db', 'playback_speed', 'low_eq_db', 'mid_eq_db', 'high_eq_db',
    'noise_reduction_percent', 'ambient_sound_mix_percent', 'fade_in_ms', 'fade_out_ms',
    'dry_wet_percent', 'reverb_wet_percent', 'mfcc_dimensions', 'snr_variation_db',
    'formant_shift_percent', 'vibrato_frequency_hz', 'vibrato_depth_percent',
    'spectrum_blind_spot_percent', 'snr_target_db', 'filter_q', 'sample_rate_hz',
    'output_bitrate_kbps', 'voice_library_id', 'high_frequency_perturbation_enabled',
    'high_frequency_perturbation_interval_ms', 'high_frequency_perturbation_strength_percent',
    'high_frequency_perturbation_level_db',
  ];
  const actualFields = [...definitions.matchAll(/key: '([^']+)'/g)].map((match) => match[1]);

  assert.deepEqual(actualFields, expectedFields);
  assert.match(formatter, /value === null[\s\S]*key === 'snr_target_db'[\s\S]*自动（源素材基线）/);
  assert.match(formatter, /typeof value === 'boolean'[\s\S]*开启[\s\S]*关闭/);
  assert.match(formatter, /typeof value === 'string'[\s\S]*natural_voice_mode[\s\S]*自然动态[\s\S]*保持原声/);
  assert.match(app, /AUDIO_VALUE_PRESETS\.map\(\(preset\) => <Checkbox/);
  assert.doesNotMatch(app, /AUDIO_VALUE_PRESETS\.filter\(\(preset\) => preset\.id !== 'p2[12]'\)/);
});

test('App 将完整视频参数快照交给 Rust 周期控制器', async () => {
  const app = await readSource('./App.tsx');
  assert.match(app, /params: mediaEffectParams,[\s\S]*min_period_ms: videoPeriodRange\.minMs[\s\S]*max_period_ms: videoPeriodRange\.maxMs/);
  assert.match(app, /configure_realtime_video_cycle/);
  assert.doesNotMatch(app, /PlannedVideoCyclePayload|buildVideoCycleSeed|applyPlannedVideoCycle/);
  assert.doesNotMatch(app, /sampleSubtleVideoParams|sampleVideoCycle|sanitizeMappedVideoSample/);
});

test('视频真实渲染只有 Rust active 指纹确认后才显示生效', async () => {
  const app = await readSource('./App.tsx');
  assert.match(app, /effectiveMediaVideoBackend/);
  assert.match(app, /\['source_transitioning', 'ready', 'applying', 'result_unknown', 'readback_confirmed', 'presented_confirmed'\]/);
  assert.doesNotMatch(app, /activeVideoRenderRef|prepare_realtime_video_plan|commit_realtime_video_plan/);
});

test('播放池组件展示真实顺序、数量和当前项，并区分整批导入与追加媒体', async () => {
  const [app, pool, css] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop/playback-pool-panel.tsx'),
    readSource('./desktop-layout.css'),
  ]);

  assert.match(app, /<PlaybackPoolPanel/);
  assert.match(app, /sources=\{sourceMediaPool\}/);
  assert.match(app, /currentIndex=\{snapshot\?\.source_media_index \?\? null\}/);
  assert.match(pool, /<List/);
  assert.match(pool, /本地有序循环 · \{sources\.length\}\/100 项/);
  assert.match(pool, /第 \{index \+ 1\} 项/);
  assert.match(pool, /当前项/);
  assert.match(pool, /导入媒体/);
  assert.match(pool, /追加媒体/);
  assert.match(pool, /<Empty/);
  assert.match(pool, /尚未导入本地媒体/);
  assert.match(pool, /整批替换/);
  assert.ok((pool.match(/onImport/g) ?? []).length >= 3);
  assert.doesNotMatch(pool, /默认播放池|<Select/);
  assert.match(css, /\.desktop-source-empty\s*\{/);
  assert.match(css, /\.desktop-source-list\s*\{/);
  assert.match(css, /\.desktop-source-item-active\s*\{/);
  assert.match(css, /\.desktop-source-current\s*>\s*\.desktop-source-list\s*\{[^}]*max-height:[^}]*overflow:\s*hidden auto/);
});

test('多支路声音快照只接受有效且受上限约束的参数', async () => {
  const capabilitySource = await readSource('./audio-processing-capabilities.ts');
  const { resolveAudioStreamBranches } = await loadTypeScriptModule('./audio-processing-capabilities.ts');
  assert.match(capabilitySource, /import \{ AUDIO_MIX_PICK_HARD_MAX \} from '\.\/runtime-parameter-scheduler'/);
  const audioDto = (inputGain, lowEq) => ({
    natural_voice_mode: 'original',
    voice_library_id: null,
    input_gain_db: inputGain,
    output_gain_db: 0,
    loudness_adjustment_db: 0,
    low_eq_db: lowEq,
    mid_eq_db: 0,
    high_eq_db: 0,
    pitch_shift_semitones: 0,
    formant_shift_percent: 0,
    playback_speed: 1,
    fade_in_ms: 0,
    fade_out_ms: 0,
    reverb_wet_percent: 0,
    noise_reduction_percent: 0,
    phase_perturbation_percent: 0,
    vibrato_frequency_hz: 5,
    vibrato_depth_percent: 0,
    environment_noise_percent: 0,
    environment_noise_dbfs: -48,
    filter_q: 1,
    dry_wet_percent: 0,
    ambient_sound_mix_percent: 0,
    spectral_perturbation_percent: 0,
    sample_rate_hz: null,
    output_bitrate_kbps: 192,
  });
  const params = audioDto(1, 0.1);
  const first = audioDto(2, 0.2);
  const second = audioDto(3, 0.3);

  assert.deepEqual(resolveAudioStreamBranches(params, [], false), []);
  assert.deepEqual(resolveAudioStreamBranches(params, [], true), [params]);
  assert.deepEqual(resolveAudioStreamBranches(params, [first, second], true), [first, second]);

  assert.deepEqual(resolveAudioStreamBranches({ ...params, input_gain_db: Number.NaN }, [], true), []);
  assert.deepEqual(resolveAudioStreamBranches({ ...params, input_gain_db: Number.POSITIVE_INFINITY }, [], true), []);
  assert.deepEqual(resolveAudioStreamBranches({ ...params, input_gain_db: '1' }, [], true), []);
  assert.deepEqual(resolveAudioStreamBranches(params, [first, { ...second, input_gain_db: Number.NaN }], true), []);
  assert.deepEqual(resolveAudioStreamBranches(params, [first, second, first, second], true), [first, second, first, second]);
  assert.deepEqual(resolveAudioStreamBranches(params, [first, second, first, second, first], true), []);
  assert.deepEqual(resolveAudioStreamBranches(params, { 0: first, length: 1 }, true), []);
});

test('运行中的声音参数卡从 Rust 快照投影，未运行时才回退编辑值', async () => {
  const app = await readSource('./App.tsx');

  assert.match(app, /audio_stream_params\?: MediaEffectParams\['audio'\] \| null/);
  assert.match(app, /audio_stream_variants\?: MediaEffectParams\['audio'\]\[\]/);
  assert.match(app, /resolveAudioStreamBranches\([\s\S]*snapshot\?\.audio_stream_params[\s\S]*snapshot\?\.audio_stream_variants/s);
  assert.match(app, /const audioDisplayParams = actualAudioBranches\[0\] \?\? mediaEffectParams\?\.audio \?\? null/);
  assert.match(app, /buildAudioCapabilityRows\([\s\S]*audioDisplayParams/s);
});

test('主窗口只暴露本期视频参数，并保留插话与固定话术抽屉', async () => {
  const app = await readSource('./App.tsx');
  const desktop = app.slice(app.indexOf('function DesktopApp()'));

  assert.match(desktop, /title="随机插话"/);
  assert.match(desktop, /title="固定话术"/);
  assert.match(desktop, /title="高级声音设置"/);
  assert.doesNotMatch(desktop, /实时话术幻化/);
});

test('三个功能抽屉复用响应式高密度结构，PortAudio 设置固定在主页', async () => {
  const [app, css, drawer, portAudio] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop-layout.css'),
    readSource('./desktop/feature-drawer.tsx'),
    readSource('./desktop/portaudio-device-panel.tsx'),
  ]);

  assert.match(drawer, /import \{ Card, Drawer, Typography \} from 'antd'/);
  assert.match(drawer, /width=\{`min\(\$\{width\}px, 100vw\)`\}/);
  assert.match(drawer, /footer=\{footer \? <div className="feature-drawer-footer">/);
  assert.match(drawer, /aria-live="polite"/);
  assert.match(drawer, /mask: \{ background: 'rgba\(0, 0, 0, 0\.64\)' \}/);
  assert.equal((app.match(/<FeatureDrawer\b/g) ?? []).length, 3);
  assert.match(app, /title="高级声音设置"[\s\S]*width=\{760\}/);
  assert.match(app, /title="随机插话"[\s\S]*width=\{560\}/);
  assert.match(app, /递归扫描本地媒体目录及其子目录/);
  assert.match(app, /title="固定话术"[\s\S]*width=\{560\}/);
  for (const label of ['插话媒体目录', '插话声音预设', '预制标题', '朗读正文', '声音参数值预设']) {
    assert.match(app, new RegExp(`label="${label}"`), `抽屉字段缺少可见标签：${label}`);
  }
  for (const label of ['Host API', '输出设备', '内存缓冲']) {
    assert.match(portAudio, new RegExp(`>${label}</label>`), `主页 PortAudio 设置缺少可见标签：${label}`);
  }
  assert.match(app, /删除这条预制文本？/);
  assert.match(app, /ariaLabel="原声压低过渡" label="原声压低过渡"/);
  assert.match(app, /ariaLabel="原声恢复过渡" label="原声恢复过渡"/);
  assert.match(app, /重新生成本周期参数/);
  assert.doesNotMatch(app, /应用声音参数/);
  assert.match(css, /feature-drawer-checkbox-grid[^}]*repeat\(4,/);
  assert.match(css, /@media\s*\(max-width:\s*900px\)[\s\S]*feature-drawer-checkbox-grid[^}]*repeat\(2,/);
  assert.match(css, /@media\s*\(max-width:\s*600px\)[\s\S]*feature-drawer-field-grid[\s\S]*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  assert.doesNotMatch(css, /\.ant-/);
});

test('页面固定状态层低于抽屉且抽屉保留滚动和键盘焦点行为', async () => {
  const [app, css, drawer, shell] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop-layout.css'),
    readSource('./desktop/feature-drawer.tsx'),
    readSource('./desktop/desktop-shell.tsx'),
  ]);

  const fixedLayerZIndexes = [
    ...css.matchAll(/\.desktop-auth-session-(?:warning|controls)\s*\{[^}]*z-index:\s*(\d+)/gs),
  ].map((match) => Number(match[1]));
  assert.deepEqual(fixedLayerZIndexes, [20, 21]);
  assert.ok(fixedLayerZIndexes.every((zIndex) => zIndex < 1000), '页面 fixed 状态层必须低于 Ant Design 弹层');
  assert.match(css, /\.desktop-topbar\s*\{[^}]*z-index:\s*3000/s);
  assert.match(shell, /export function DesktopWindowFrame/);
  assert.match(drawer, /export const DESKTOP_DRAWER_ROOT_STYLE/);
  assert.match(drawer, /rootStyle=\{DESKTOP_DRAWER_ROOT_STYLE\}/);
  assert.equal((app.match(/rootStyle=\{DESKTOP_DRAWER_ROOT_STYLE\}/g) ?? []).length, 2);
  assert.match(drawer, /\sautoFocus\s/);
  assert.match(drawer, /\skeyboard\s/);
  assert.match(drawer, /header:\s*\{[^}]*flex:\s*'0 0 auto'/s);
  assert.match(drawer, /body:\s*\{[^}]*minHeight:\s*0[^}]*overflowY:\s*'auto'/s);
  assert.match(drawer, /footer:\s*\{[^}]*flex:\s*'0 0 auto'/s);
  assert.match(css, /@media\s*\(max-width:\s*600px\)[\s\S]*feature-drawer-field-grid[\s\S]*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  assert.doesNotMatch(css, /\.ant-/);
});

test('顶栏使用真实图标资源和安全的 Tauri 窗口动作', async () => {
  const [shell, accessPanel, capabilitySource, tauriConfigSource] = await Promise.all([
    readSource('./desktop/desktop-shell.tsx'),
    readSource('./desktop/control-plane-access-panel.tsx'),
    readSource('../../src-tauri/capabilities/default.json'),
    readSource('../../src-tauri/tauri.conf.json'),
  ]);
  const capability = JSON.parse(capabilitySource);
  const tauriConfig = JSON.parse(tauriConfigSource);

  assert.match(shell, /src="\/app-icon\.png"/);
  assert.match(shell, />GpAutoLive<\/span>/);
  assert.match(accessPanel, /alt="GpAutoLive 产品 Logo"/);
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
