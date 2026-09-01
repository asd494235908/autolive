import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

async function readSource(path) {
  return readFile(new URL(path, import.meta.url), 'utf8');
}

test('主页同步原型信息架构但保留旧设置与状态路由', async () => {
  const [app, shell] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop/desktop-shell.tsx'),
  ]);

  assert.match(shell, /const DESKTOP_ROUTE_ITEMS = \[\s*\{ path: '\/', label: '主页' \},\s*\]/);
  assert.match(app, /<Route path="\/settings" element=\{<DesktopApp \/>\} \/>/);
  assert.match(app, /<Route path="\/status" element=\{<DesktopApp \/>\} \/>/);
});

test('主页主体按播放区、音频左列、画面右列和输出状态区组织', async () => {
  const [app, css] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop-layout.css'),
  ]);
  const home = app.slice(app.indexOf('<DesktopShell>'), app.indexOf('<FeatureDrawer'));

  const source = home.indexOf('area="source"');
  const media = home.indexOf('area="media"');
  const output = home.indexOf('area="output"');
  assert.ok(source >= 0 && source < media && media < output);

  const mediaColumn = home.slice(home.indexOf('<DesktopColumn area="media"'), home.indexOf('<DesktopColumn area="output"'));
  const audioColumnAt = mediaColumn.indexOf('desktop-media-domain-lane--audio');
  const videoColumnAt = mediaColumn.indexOf('desktop-media-domain-lane--video');
  assert.ok(audioColumnAt >= 0 && audioColumnAt < videoColumnAt);
  const audioColumn = mediaColumn.slice(audioColumnAt, videoColumnAt);
  const videoColumn = mediaColumn.slice(videoColumnAt);
  assert.ok(audioColumn.indexOf('title="音频"') < audioColumn.indexOf('title="插话声音预设"'));
  assert.match(audioColumn, /titleIcon=\{<SoundOutlined \/>\}[\s\S]*subtitle="插话与普通声音参数"/);
  assert.match(videoColumn, /title="画面"/);
  assert.match(videoColumn, /titleIcon=\{<PictureOutlined \/>\}[\s\S]*subtitle="视频周期与视觉参数"/);
  assert.doesNotMatch(mediaColumn, /title="插话音频"|title="视频"|独立声音周期与预设参数变化/);
  assert.match(css, /\.desktop-column-media\s*\{[^}]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/s);
  assert.doesNotMatch(home, /音视频同时应用|title="周期变化"|DesktopStatusStrip/);
});

test('插话周期并入音频卡，声音事实三列横排且每项名称和值左右排列', async () => {
  const [app, statusPanel, css] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop/audio-processing-panel.tsx'),
    readSource('./desktop-layout.css'),
  ]);
  const mediaColumn = app.slice(app.indexOf('<DesktopColumn area="media"'), app.indexOf('<DesktopColumn area="output"'));
  const audioColumn = mediaColumn.slice(mediaColumn.indexOf('desktop-media-domain-lane--audio'), mediaColumn.indexOf('desktop-media-domain-lane--video'));
  const audioCard = audioColumn.slice(audioColumn.indexOf('title="音频"'), audioColumn.indexOf('title="插话声音预设"'));
  const interludeCycleAt = audioCard.indexOf('title="插话声音周期"');
  const interludeFactsAt = audioCard.indexOf('ariaLabel="插话音频状态"');
  const ordinaryCycleAt = audioCard.indexOf('title="声音周期"');

  assert.ok(interludeCycleAt >= 0 && interludeCycleAt < interludeFactsAt && interludeFactsAt < ordinaryCycleAt);
  assert.equal((audioCard.match(/<AudioProcessingPanel/g) ?? []).length, 2);
  for (const label of ['实际出口', '处理状态', '当前预设']) assert.match(statusPanel, new RegExp(`<dt>${label}</dt>`));
  assert.match(css, /\.desktop-audio-facts\s*\{[^}]*display:\s*grid[^}]*grid-template-columns:\s*repeat\(3,\s*minmax\(0,\s*1fr\)\)/s);
  assert.match(css, /\.desktop-audio-facts\s*>\s*div\s*\{[^}]*display:\s*flex[^}]*align-items:\s*center[^}]*justify-content:\s*space-between/s);
});

test('主页插话声音周期显示已保存的随机插话触发周期', async () => {
  const [app, cycleCard] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop/media-cycle-card.tsx'),
  ]);
  const periodSource = app.slice(
    app.indexOf('const interludePeriodRange'),
    app.indexOf('const interludeStatusLabel'),
  );
  const interludeCard = app.slice(
    app.indexOf('title="插话声音周期"'),
    app.indexOf('ariaLabel="插话音频状态"'),
  );

  assert.match(periodSource, /snapshot\?\.interlude\?\.interval_min_ms \?\? 8_000/);
  assert.match(periodSource, /snapshot\?\.interlude\?\.interval_max_ms \?\? 13_000/);
  assert.match(periodSource, /const interludePeriodRange[\s\S]*interval_(?:min|max)_ms/);
  assert.match(interludeCard, /range=\{interludePeriodRange\}/);
  assert.match(cycleCard, /formatCycleSeconds\(range\.minMs\)[\s\S]*formatCycleSeconds\(range\.maxMs\)/);
  assert.doesNotMatch(cycleCard, /range\.(?:minMs|maxMs)[^\n]*toFixed\(0\)/);
});

test('插话播放时上下波形互斥，普通声音保持直线', async () => {
  const app = await readSource('./App.tsx');

  assert.match(app, /const interludePlaybackActive = Boolean\(\s*interludeDraft\.enabled[\s\S]*snapshot\?\.interlude\?\.enabled[\s\S]*interludeRuntime\?\.status === 'playing'[\s\S]*\);/);
  assert.match(app, /drawDiagnosticCanvas\([\s\S]*lowFrequencyLineCanvasRef\.current,[\s\S]*interludePlaybackActive \? \[0, 0\] : diagnosticMessage\?\.line \?\? \[0, 0\],/);
  assert.match(app, /const interludeWaveformActive = Boolean\([\s\S]*interludePlaybackActive[\s\S]*diagnosticFresh[\s\S]*diagnosticMessage\.sent_at_ms >= interludeRuntime\.started_at_ms[\s\S]*\);/);
  assert.match(app, /const interludeWaveform = interludeWaveformActive[\s\S]*\? diagnosticMessage\?\.line \?\? \[\][\s\S]*: \[\];/);
  assert.match(app, /points=\{interludeWaveform\.map/);
  assert.match(app, /aria-label=\{interludeWaveformActive[\s\S]*插话混入后的最终音频诊断波形[\s\S]*当前没有可用的插话最终混音诊断波形/);
  assert.match(app, /aria-label=\{interludePlaybackActive \? '插话播放中，普通声音波形为直线' : '声音波形'\}/);
});

test('插话声音预设独立成卡，音视频操作位于标题且声音入口移到右栏', async () => {
  const app = await readSource('./App.tsx');
  const home = app.slice(app.indexOf('<DesktopShell>'), app.indexOf('<FeatureDrawer'));
  const mediaColumn = home.slice(home.indexOf('<DesktopColumn area="media"'), home.indexOf('<DesktopColumn area="output"'));
  const audioColumn = mediaColumn.slice(mediaColumn.indexOf('desktop-media-domain-lane--audio'), mediaColumn.indexOf('desktop-media-domain-lane--video'));
  const audioCard = audioColumn.slice(audioColumn.indexOf('title="音频"'), audioColumn.indexOf('title="插话声音预设"'));
  const presetCard = audioColumn.slice(audioColumn.indexOf('title="插话声音预设"'));
  const videoCard = mediaColumn.slice(mediaColumn.indexOf('title="画面"'));
  const output = home.slice(home.indexOf('<DesktopColumn area="output"'));

  assert.match(presetCard, /activeInterludePresets\[0\][\s\S]*desktop-preset-parameter-grid[\s\S]*AUDIO_PRESET_FIELD_DEFINITIONS\.map/);
  assert.doesNotMatch(audioCard, /AUDIO_PRESET_FIELD_DEFINITIONS\.map|desktop-preset-parameter-grid|>高级设置<|>插话文件</);
  assert.match(audioCard, /extra=\{[\s\S]*普通声音处理[\s\S]*aria-label="声音处理"/);
  assert.match(videoCard, /extra=\{[\s\S]*视频处理[\s\S]*aria-label="视频处理"[\s\S]*恢复默认/);
  assert.doesNotMatch(videoCard, />应用处理</);
  const finalAt = output.indexOf('title="最终效果窗口"');
  const featuresAt = output.indexOf('title="声音功能"');
  const portAudioAt = output.indexOf('<PortAudioDevicePanel');
  const speechAt = output.indexOf('title="话术功能"');
  assert.ok(finalAt >= 0 && finalAt < featuresAt && featuresAt < portAudioAt && portAudioAt < speechAt);
  assert.match(output, /setAudioSettingsDrawerOpen\(true\)[\s\S]*>模式修改</);
  assert.match(output, /setInterludeDrawerOpen\(true\)[\s\S]*>插话文件</);
  assert.doesNotMatch(output, />高级设置</);
});

test('PortAudio 位于话术功能上方且固定话术完整抽屉继续存在', async () => {
  const app = await readSource('./App.tsx');
  const home = app.slice(app.indexOf('<DesktopShell>'), app.indexOf('<FeatureDrawer'));
  const advanced = app.slice(app.indexOf('title="高级声音设置"'), app.indexOf('title={`当前声音预设'));

  const portAudioAt = home.indexOf('<PortAudioDevicePanel');
  const speechAt = home.indexOf('title="话术功能"');
  assert.ok(portAudioAt >= 0 && portAudioAt < speechAt);
  assert.match(home, /setFixedSpeechDrawerOpen\(true\)/);
  assert.doesNotMatch(advanced, /title="PortAudio 设备"/);
  assert.match(app, /title="固定话术"[\s\S]*删除预制文本[\s\S]*停止朗读[\s\S]*播放当前文案/);
});

test('模式修改和随机插话抽屉同步最新设计稿内容', async () => {
  const [app, css] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop-layout.css'),
  ]);
  const interruption = app.slice(app.indexOf('title="插话文件"'), app.indexOf('title="固定话术"'));
  const advanced = app.slice(app.indexOf('title="高级声音设置"'), app.indexOf('title={`当前插话预设'));

  for (const label of [
    '启用状态', '插话随机周期', '最小周期', '最大周期', '媒体来源', '独立声音轨',
    '音轨选择方式', '随机多轨合一', '插话声音预设', '抽样方式', '音量与混音',
    '请检查插话配置', '保存插话配置',
  ]) assert.match(interruption, new RegExp(label), `随机插话抽屉缺少：${label}`);
  const enabledAt = interruption.indexOf('title="启用状态"');
  const cycleAt = interruption.indexOf('title="插话随机周期"');
  const sourceAt = interruption.indexOf('title="媒体来源"');
  assert.ok(enabledAt >= 0 && enabledAt < cycleAt && cycleAt < sourceAt);
  assert.match(interruption, /ariaLabel="插话随机周期最小值（秒）"/);
  assert.match(interruption, /ariaLabel="插话随机周期最大值（秒）"/);
  assert.doesNotMatch(interruption, /<strong>触发间隔<\/strong>/);
  assert.match(interruption, /disabled=\{interludeValidationErrors\.length > 0\}/);
  assert.match(interruption, /className="interlude-compact-save"[\s\S]*onClick=\{\(\) => void saveInterludeConfig\(\)\}/);
  assert.match(css, /\.interlude-compact-save\s*\{[^}]*display:\s*none/s);
  assert.match(css, /@media \(max-height:\s*720px\)\s*\{[\s\S]*\.interlude-compact-save\s*\{[^}]*display:\s*inline-flex/s);

  for (const label of [
    '处理与输出', '多轨与预设', '已选 \{audioValuePresetIds\.length\} 项',
    '选择默认 20 项', '全选 22 项', '音频参数状态', '缓存管理',
    '重新生成本周期参数',
  ]) assert.match(advanced, new RegExp(label), `模式修改抽屉缺少：${label}`);
  assert.doesNotMatch(advanced, /恢复全部默认/);
});

test('普通声音默认 3–5 秒且插话周期在界面以秒编辑', async () => {
  const [scheduler, app] = await Promise.all([
    readSource('./runtime-parameter-scheduler.ts'),
    readSource('./App.tsx'),
  ]);
  const interruptionDrawer = app.slice(app.indexOf('title="插话文件"'), app.indexOf('title="固定话术"'));

  assert.match(scheduler, /DEFAULT_AUDIO_PERIOD_MIN_MS = 3_000/);
  assert.match(scheduler, /DEFAULT_AUDIO_PERIOD_MAX_MS = 5_000/);
  assert.match(interruptionDrawer, /ariaLabel="插话变化周期最小值（秒）"/);
  assert.match(interruptionDrawer, /ariaLabel="插话变化周期最大值（秒）"/);
  assert.doesNotMatch(interruptionDrawer, /ariaLabel="插话变化周期(?:最小|最大)值" unit="ms"/);
});

test('参数卡使用单媒体全宽密集网格，固定视觉频段拆成独立参数行', async () => {
  const [component, css] = await Promise.all([
    readSource('./media-parameter-panels/MediaParameterPanels.tsx'),
    readSource('./media-parameter-panels/media-parameter-panels.css'),
  ]);

  assert.doesNotMatch(component, /<Progress/);
  assert.match(component, /sections = \['video', 'advanced', 'audio'\]/);
  assert.match(component, /VISUAL_BAND_FREQUENCIES_HZ\.map/);
  assert.match(component, /groupedDefinitions\.flatMap/);
  assert.match(component, /const visibleParameterCount = MEDIA_PARAMETER_DEFINITIONS\[section\]\.reduce/);
  assert.match(component, /definition\.kind === 'band-weights' \? VISUAL_BAND_FREQUENCIES_HZ\.length : 1/);
  assert.match(component, /\{visibleParameterCount\} 项 · 只读快照/);
  assert.doesNotMatch(component, /media-parameter-section__group/);
  assert.match(css, /\.media-parameter-panels__columns\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/s);
  assert.match(css, /\.media-parameter-panels__columns--split\s*\{[^}]*repeat\(2,\s*minmax\(0,\s*1fr\)\)/s);
  assert.match(css, /\.media-parameter-section__grid\s*\{[^}]*repeat\(auto-fit,\s*minmax\(min\(126px,\s*100%\),\s*1fr\)\)/s);
  assert.match(css, /\.media-parameter-card\s*\{[^}]*height:\s*28px/s);
  assert.doesNotMatch(component, /MEDIA_PARAMETER_STATUS_LABELS\[status\]/);
});

test('插话卡接收最终效果窗上报的本段实际声音预设', async () => {
  const app = await readSource('./App.tsx');

  assert.match(app, /type InterludeRuntimeMessage =/);
  assert.match(app, /type: 'interlude-runtime'/);
  assert.match(app, /presetIds: audioCycle\.presetIds/);
  assert.match(app, /isInterludeRuntimeMessage\(event\.data\)/);
  assert.match(app, /clearInterludePlayback\(\{ resetSchedule: true \}\);\s*publishInterludeRuntime\('failed', \{ error: message \}\)/);
  assert.match(app, /当前插话预设/);
});

test('播放池在主页内联管理且媒体卡操作栏同步原型结构', async () => {
  const [app, playbackPool] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop/playback-pool-panel.tsx'),
  ]);
  const home = app.slice(app.indexOf('<DesktopShell>'), app.indexOf('<FeatureDrawer'));

  assert.doesNotMatch(playbackPool, /<Drawer|播放池管理/);
  assert.match(playbackPool, /追加媒体/);
  assert.match(playbackPool, /整批替换/);
  assert.match(playbackPool, /拖放媒体到此处追加 · 追加后最多可保留 100 项 · 拖动条目排序/);
  assert.match(playbackPool, /清空播放池/);
  assert.match(home, /desktop-playback-facts/);
  assert.doesNotMatch(home, /desktop-domain-actions/);
  assert.match(home, /普通声音处理/);
  assert.match(home, /aria-label="输出查看模式"/);
  assert.match(home, /label: '画中画'[^]*label: '独立窗口'/);
  assert.doesNotMatch(home, /desktop-diagnostic-grid/);
  assert.doesNotMatch(home, /audioControls=/);
  assert.match(app, /title="声音来源与自动基线"/);
});

test('播放进度位于动作按钮前且最终窗口按连接状态显示打开或聚焦', async () => {
  const app = await readSource('./App.tsx');
  const playback = app.slice(app.indexOf('title="播放控制"'), app.indexOf('</DesktopColumn>', app.indexOf('title="播放控制"')));
  const output = app.slice(app.indexOf('title="最终效果窗口"'), app.indexOf('title="声音功能"'));

  assert.ok(playback.indexOf('aria-label="播放进度"') < playback.indexOf('desktop-playback-actions'));
  assert.match(output, /onClick=\{\(\) => void openFinalEffectWindowFromHome\(\)\}>\{diagnosticFresh \? '聚焦' : '打开'\}<\/Button>/);
});

test('真实空状态不冒充音频素材、PortAudio 故障或上轮声音预设', async () => {
  const app = await readSource('./App.tsx');

  assert.match(app, /status=\{!currentSource \? '等待导入' : !currentMediaIsVideo \? '当前音频素材不适用'/);
  assert.match(app, /processingStatus=\{!currentSource \? '等待媒体'/);
  assert.match(app, /if \(!audioProcessingEnabled \|\| !snapshot\?\.source_media\)[\s\S]*audioCycleSampleRef\.current = null;[\s\S]*setAudioActivePresetIds\(\[\]\);/);
});

test('右栏顶部功能卡使用设计稿紧凑正文', async () => {
  const [app, panel, css] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop/desktop-panel.tsx'),
    readSource('./desktop-layout.css'),
  ]);

  assert.match(app, /className="desktop-output-panel desktop-compact-panel"/);
  assert.match(app, /title="声音功能" className="desktop-compact-panel"/);
  assert.match(panel, /className\.includes\('desktop-compact-panel'\) \? 8 : 12/);
  assert.match(css, /\.desktop-output-panel\s*\{[^}]*min-height:\s*106px/s);
});
