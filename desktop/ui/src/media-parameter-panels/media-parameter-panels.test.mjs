import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

async function importTypeScriptModule(relativePath) {
  const source = await readFile(new URL(relativePath, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.ESNext,
      target: ts.ScriptTarget.ES2022,
    },
    fileName: relativePath,
    reportDiagnostics: true,
  });
  assert.deepEqual(compiled.diagnostics ?? [], []);
  return import(`data:text/javascript;base64,${Buffer.from(compiled.outputText).toString('base64')}`);
}

function getTypeScriptInterfaceFields(source, interfaceName) {
  const sourceFile = ts.createSourceFile('media-parameter-types.ts', source, ts.ScriptTarget.Latest, true);
  const declaration = sourceFile.statements.find(
    (statement) => ts.isInterfaceDeclaration(statement) && statement.name.text === interfaceName,
  );
  assert.ok(declaration, `missing TypeScript interface ${interfaceName}`);
  return declaration.members.map((member) => member.name.getText(sourceFile));
}

function getRustStructFields(source, structName) {
  const match = source.match(new RegExp(`pub struct ${structName} \\{([\\s\\S]*?)\\n\\}`));
  assert.ok(match, `missing Rust struct ${structName}`);
  return [...match[1].matchAll(/pub\s+([a-z0-9_]+)\s*:/g)].map((field) => field[1]);
}

function getJsxElementsWithAttribute(source, fileName, attributeName) {
  const sourceFile = ts.createSourceFile(
    fileName,
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const elements = [];
  const visit = (node) => {
    if (ts.isJsxSelfClosingElement(node) || ts.isJsxOpeningElement(node)) {
      const hasAttribute = node.attributes.properties.some((property) => (
        ts.isJsxAttribute(property) && property.name.getText(sourceFile) === attributeName
      ));
      if (hasAttribute) {
        elements.push((ts.isJsxOpeningElement(node) ? node.parent : node).getText(sourceFile));
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(sourceFile);
  return elements;
}

const READ_ONLY_RANDOMIZED_AUDIO_PARAMETERS = [
  ['pitch_shift_semitones', '高质量音高'],
  ['playback_speed', '播放速度'],
  ['formant_shift_percent', '共振峰偏移'],
  ['mfcc_shift_percent', 'MFCC 偏移'],
  ['mfcc_dimensions', 'MFCC 维度'],
  ['snr_variation_db', 'SNR 浮动'],
  ['spectrum_blind_spot_percent', '频谱盲区宽度'],
  ['dry_wet_percent', '干湿比'],
  ['ambient_sound_mix_percent', '环境声素材混合'],
];

test('字段定义完整覆盖当前正式 Rust/TypeScript 参数契约', async () => {
  const definitions = await importTypeScriptModule('./parameter-definitions.ts');
  const typeSource = await readFile(new URL('./media-parameter-types.ts', import.meta.url), 'utf8');
  const rustSource = await readFile(
    new URL('../../../src-tauri/src/media_effect_params.rs', import.meta.url),
    'utf8',
  );

  for (const [section, interfaceName, rustStructName] of [
    ['audio', 'AudioEffectParams', 'AudioEffectParams'],
    ['video', 'VideoEffectParams', 'VideoEffectParams'],
    ['advanced', 'AdvancedEffectParams', 'AdvancedEffectParams'],
  ]) {
    const definitionFields = definitions.MEDIA_PARAMETER_DEFINITIONS[section]
      .map((definition) => definition.field)
      .sort();
    const typeFields = getTypeScriptInterfaceFields(typeSource, interfaceName).sort();
    const rustFields = getRustStructFields(rustSource, rustStructName).sort();
    assert.deepEqual(typeFields, rustFields, `${section} TypeScript 契约必须与 Rust 同步`);
    assert.deepEqual(definitionFields, typeFields, `${section} 每个正式字段必须恰好有一个面板定义`);
  }
});

test('状态、频段和关键真实链路声明保持稳定', async () => {
  const definitions = await importTypeScriptModule('./parameter-definitions.ts');
  assert.deepEqual(definitions.MEDIA_PARAMETER_STATUS_LABELS, {
    implemented: '已接入',
    planned: '正式需求待实现',
    pending_confirmation: '待确认',
  });
  assert.deepEqual(definitions.VISUAL_BAND_FREQUENCIES_HZ, [
    65, 92, 131, 188, 267, 381, 544, 777, 1110, 1585, 2263, 20_000,
  ]);

  const all = Object.values(definitions.MEDIA_PARAMETER_DEFINITIONS).flat();
  const statusByPath = new Map(all.map((definition) => [
    `${definition.section}.${definition.field}`,
    definition.status,
  ]));
  assert.equal(statusByPath.get('audio.pitch_shift_semitones'), 'implemented');
  assert.equal(statusByPath.get('audio.formant_shift_percent'), 'implemented');
  assert.equal(statusByPath.get('audio.playback_speed'), 'implemented');
  assert.equal(statusByPath.get('audio.spectral_perturbation_percent'), 'implemented');
  assert.equal(statusByPath.get('audio.high_frequency_perturbation_enabled'), 'implemented');
  assert.equal(statusByPath.get('audio.current_formant_hz'), 'implemented');
  assert.equal(statusByPath.get('video.brightness_percent'), 'implemented');
  assert.equal(statusByPath.get('video.frame_rate_jitter_percent'), 'implemented');
  assert.equal(statusByPath.get('video.edge_softness_percent'), 'implemented');
  assert.equal(statusByPath.get('advanced.picture_in_picture_enabled'), 'implemented');
  assert.equal(statusByPath.get('advanced.local_blur_enabled'), 'implemented');
  assert.equal(statusByPath.get('advanced.band_weights'), 'implemented');
  assert.ok(all.every(({ status }) => status === 'implemented'));
  assert.ok(all.every(({ field }) => !/research|ocr/i.test(field)));
});

test('分段进度按完整参数范围归一化并夹紧边界', async () => {
  const progress = await importTypeScriptModule('./media-parameter-progress.ts');

  assert.equal(progress.normalizeMediaParameterProgress(-100, -100, 100), 0);
  assert.equal(progress.normalizeMediaParameterProgress(0, -100, 100), 50);
  assert.equal(progress.normalizeMediaParameterProgress(100, -100, 100), 100);
  assert.equal(progress.normalizeMediaParameterProgress(-200, -100, 100), 0);
  assert.equal(progress.normalizeMediaParameterProgress(200, -100, 100), 100);
  assert.equal(progress.normalizeMediaParameterProgress(null, -100, 100), 0);
  assert.equal(progress.normalizeMediaParameterProgress(Number.NaN, -100, 100), 0);
  assert.equal(progress.normalizeMediaParameterProgress(1, 1, 1), 0);
});

test('参数状态面板保持只读，声音编辑通过独立插槽组合', async () => {
  const componentSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');
  const typeSource = await readFile(new URL('./media-parameter-types.ts', import.meta.url), 'utf8');
  const cssSource = await readFile(new URL('./media-parameter-panels.css', import.meta.url), 'utf8');
  const appSource = await readFile(new URL('../App.tsx', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(componentSource, {
    compilerOptions: {
      jsx: ts.JsxEmit.ReactJSX,
      module: ts.ModuleKind.ESNext,
      target: ts.ScriptTarget.ES2022,
    },
    fileName: 'MediaParameterPanels.tsx',
    reportDiagnostics: true,
  });

  assert.deepEqual(compiled.diagnostics ?? [], []);
  for (const control of ['Input', 'InputNumber', 'Slider', 'Select', 'Switch', 'Tabs']) {
    assert.doesNotMatch(componentSource, new RegExp(`\\b${control}\\b`), `只读面板不得使用 ${control}`);
  }
  assert.doesNotMatch(componentSource, /<(?:input|select|textarea)\b/i);
  assert.doesNotMatch(componentSource, /\bonChange\b/);
  assert.doesNotMatch(componentSource, /\bupdateMediaParameterValue\b/);
  const panelInvocation = appSource.slice(
    appSource.indexOf('<MediaParameterPanels'),
    appSource.indexOf('/>', appSource.indexOf('<MediaParameterPanels')),
  );
  assert.doesNotMatch(panelInvocation, /\bonChange=/);
  assert.match(
    componentSource,
    /params\[definition\.section\]\[definition\.field\]|readMediaParameterValue\(/,
  );
  assert.match(componentSource, /<Alert/);
  assert.match(componentSource, /<Spin/);
  assert.match(componentSource, /aria-labelledby/);
  assert.doesNotMatch(componentSource, /aria-describedby/);
  assert.match(componentSource, /<Progress\b[\s\S]*?aria-label=/);
  assert.doesNotMatch(componentSource, /role="progressbar"/);
  assert.doesNotMatch(cssSource, /\.ant-/);
});

test('主页声音控件使用明确受控契约并只开放保留的声音设置', async () => {
  const componentSource = await readFile(new URL('./AudioParameterControls.tsx', import.meta.url), 'utf8');
  const typeSource = await readFile(new URL('./media-parameter-types.ts', import.meta.url), 'utf8');
  const indexSource = await readFile(new URL('./index.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(componentSource, {
    compilerOptions: {
      jsx: ts.JsxEmit.ReactJSX,
      module: ts.ModuleKind.ESNext,
      target: ts.ScriptTarget.ES2022,
    },
    fileName: 'AudioParameterControls.tsx',
    reportDiagnostics: true,
  });

  assert.deepEqual(compiled.diagnostics ?? [], []);
  assert.match(typeSource, /interface AudioParameterControlsProps/);
  assert.match(typeSource, /value:\s*AudioEffectParams/);
  assert.match(typeSource, /disabled:\s*boolean/);
  assert.match(typeSource, /ambientSoundPath:\s*string\s*\|\s*null/);
  assert.match(typeSource, /type EditableAudioParameterField\s*=/);
  assert.match(typeSource, /onChange:\s*<Field extends EditableAudioParameterField>/);
  assert.match(typeSource, /onChooseAmbientSound:\s*\(\)\s*=>\s*void\s*\|\s*Promise<void>/);
  assert.match(typeSource, /onClearAmbientSound:\s*\(\)\s*=>\s*void/);
  assert.match(indexSource, /export \{ AudioParameterControls \} from '\.\/AudioParameterControls'/);

  for (const field of [
    'natural_voice_mode',
    'voice_library_id',
    'snr_target_db',
  ]) {
    assert.match(componentSource, new RegExp(field), `缺少声音字段 ${field}`);
  }
  assert.match(componentSource, /\bForm\b/);
  assert.match(componentSource, /\bInputNumber\b/);
  assert.match(componentSource, /\bSelect\b/);
  assert.match(componentSource, /\bdisabled=\{disabled\}/);
});

test('九项随机声音参数只通过统一参数卡展示，不提供手动编辑控件', async () => {
  const definitions = await importTypeScriptModule('./parameter-definitions.ts');
  const controlsSource = await readFile(new URL('./AudioParameterControls.tsx', import.meta.url), 'utf8');
  const panelSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');
  const audioDefinitions = new Map(
    definitions.MEDIA_PARAMETER_DEFINITIONS.audio.map((definition) => [definition.field, definition]),
  );
  const editableControls = getJsxElementsWithAttribute(
    controlsSource,
    'AudioParameterControls.tsx',
    'onChange',
  );

  assert.match(panelSource, /MEDIA_PARAMETER_DEFINITIONS\[section\]/);
  assert.match(panelSource, /params\[definition\.section\]\[definition\.field\]/);
  for (const [field, label] of READ_ONLY_RANDOMIZED_AUDIO_PARAMETERS) {
    const definition = audioDefinitions.get(field);
    assert.ok(definition, `${label} 必须继续由统一只读参数面板展示`);
    assert.equal(definition.label, label);
    assert.equal(
      editableControls.some((control) => new RegExp(`\\b${field}\\b`).test(control)),
      false,
      `${label} 不得绑定手动编辑事件`,
    );
  }
});

test('目标信噪比和环境声来源提供可见、可访问且可恢复的状态', async () => {
  const componentSource = await readFile(new URL('./AudioParameterControls.tsx', import.meta.url), 'utf8');
  const definitionsSource = await readFile(new URL('./parameter-definitions.ts', import.meta.url), 'utf8');

  assert.match(componentSource, /label="目标信噪比"/);
  assert.match(componentSource, /aria-label="目标信噪比"/);
  assert.match(componentSource, /aria-describedby=\{snrTargetHelpId\}/);
  assert.match(componentSource, /aria-invalid=\{Boolean\(snrTargetError\)\}/);
  assert.match(componentSource, /placeholder="自动"/);
  assert.match(componentSource, />自动<\/Button>/);
  assert.match(componentSource, /label="环境声素材"/);
  assert.match(componentSource, /aria-label="环境声素材路径"/);
  assert.match(componentSource, /aria-describedby=\{ambientSoundHelpId\}/);
  assert.match(componentSource, /ambientSoundPath\s*\?\s*'用户素材'\s*:\s*'内置环境声'/);
  assert.match(componentSource, /ambientSoundPath\s*\?\?\s*'内置环境声'/);
  assert.match(componentSource, /onChooseAmbientSound/);
  assert.match(componentSource, /onClearAmbientSound/);
  assert.doesNotMatch(componentSource, /validateStatus=\{ambientSelection|role="alert"[^>]*>环境声混合已开启/);
  assert.match(componentSource, /用户素材可选覆盖/);
  assert.match(definitionsSource, /用户素材可选覆盖/);
});

test('声音控件插槽位于声音列且窄容器单列无横向滚动', async () => {
  const componentSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');
  const typeSource = await readFile(new URL('./media-parameter-types.ts', import.meta.url), 'utf8');
  const cssSource = await readFile(new URL('./media-parameter-panels.css', import.meta.url), 'utf8');

  assert.match(typeSource, /audioControls\?:\s*ReactNode/);
  assert.match(componentSource, /audioControls[\s\S]*?media-parameter-panels__lane--audio[\s\S]*?\{audioControls/);
  assert.match(componentSource, /media-parameter-panels__audio-controls/);
  assert.match(cssSource, /\.audio-parameter-controls\s*\{[^}]*width:\s*100%[^}]*min-width:\s*0[^}]*container-type:\s*inline-size/s);
  assert.match(
    cssSource,
    /\.audio-parameter-controls__field-grid[\s\S]*?grid-template-columns:\s*repeat\(auto-fit,\s*minmax\(min\(/s,
  );
  assert.match(cssSource, /@container\s*\(max-width:\s*520px\)[\s\S]*?grid-template-columns:\s*minmax\(0,\s*1fr\)/s);
  assert.doesNotMatch(cssSource, /overflow-x:\s*(?:auto|scroll)/);
  assert.doesNotMatch(cssSource, /\.ant-/);
});

test('数值参数使用 Ant Design 分段进度，视频与声音参数保持双主栏', async () => {
  const componentSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');
  const cssSource = await readFile(new URL('./media-parameter-panels.css', import.meta.url), 'utf8');

  assert.match(componentSource, /import\s*\{[\s\S]*?\bProgress\b[\s\S]*?\}\s*from 'antd'/);
  assert.match(componentSource, /<Progress\b[\s\S]*?\bpercent=/);
  assert.match(componentSource, /<Progress\b[\s\S]*?\bsteps=/);
  for (const label of ['普通视频', '普通声音', '高级视觉']) {
    assert.match(componentSource, new RegExp(`['"]${label}['"]`));
  }
  assert.match(componentSource, /className="media-parameter-panels__columns"/);
  assert.match(componentSource, /className="media-parameter-panels__lane media-parameter-panels__lane--video"/);
  assert.match(componentSource, /className="media-parameter-panels__lane media-parameter-panels__lane--audio"/);
  assert.match(
    componentSource,
    /media-parameter-panels__lane--video"[\s\S]*?section="video"[\s\S]*?section="advanced"[\s\S]*?media-parameter-panels__lane--audio"[\s\S]*?section="audio"/,
  );
  assert.match(
    cssSource,
    /\.media-parameter-panels__columns\s*\{[^}]*display:\s*grid[^}]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/s,
  );
});

test('分段进度使用公开紧凑尺寸数组，并使用参考图提取的五色色板', async () => {
  const componentSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');
  const cssSource = await readFile(new URL('./media-parameter-panels.css', import.meta.url), 'utf8');
  const feedback = await importTypeScriptModule('./parameter-panel-feedback.ts');
  const sourceFile = ts.createSourceFile(
    'MediaParameterPanels.tsx',
    componentSource,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const progressElements = [];
  const visit = (node) => {
    if (ts.isJsxSelfClosingElement(node) && node.tagName.getText(sourceFile) === 'Progress') {
      progressElements.push(node);
    }
    ts.forEachChild(node, visit);
  };
  visit(sourceFile);

  assert.ok(progressElements.length >= 2, '普通数值和频段权重都必须使用 Progress');
  for (const element of progressElements) {
    const sizeAttribute = element.attributes.properties.find(
      (property) => ts.isJsxAttribute(property) && property.name.getText(sourceFile) === 'size',
    );
    assert.ok(sizeAttribute && sizeAttribute.initializer && ts.isJsxExpression(sizeAttribute.initializer));
    const sizeExpression = sizeAttribute.initializer.expression;
    assert.ok(sizeExpression && ts.isArrayLiteralExpression(sizeExpression), 'Progress size 必须使用公开数组 API');
    assert.equal(sizeExpression.elements.length, 2);
    assert.ok(sizeExpression.elements.every(ts.isNumericLiteral));
    const [unitWidth, height] = sizeExpression.elements.map((element) => Number(element.text));
    assert.ok(unitWidth >= 4 && unitWidth <= 8, '单个分段必须在可读前提下收窄');
    assert.ok(height >= 4 && height <= 8, '分段高度必须兼顾紧凑与可读性');

    const stepsAttribute = element.attributes.properties.find(
      (property) => ts.isJsxAttribute(property) && property.name.getText(sourceFile) === 'steps',
    );
    assert.ok(stepsAttribute && stepsAttribute.initializer && ts.isJsxExpression(stepsAttribute.initializer));
    const stepsExpression = stepsAttribute.initializer.expression;
    assert.ok(stepsExpression && ts.isNumericLiteral(stepsExpression), 'Progress steps 必须是可审计的固定段数');
    const stepCount = Number(stepsExpression.text);
    assert.ok(
      stepCount * unitWidth + (stepCount - 1) * 2 <= 96,
      '分段总宽度必须适配 960px 最小窗口下的卡片正文',
    );
  }

  assert.deepEqual(feedback.REFERENCE_IMAGE_ACCENT_COLORS, {
    cyan: '#38B8F8',
    purple: '#C888F8',
    magenta: '#E878F8',
    orange: '#F89838',
    yellow: '#F8C818',
  });
  assert.match(componentSource, /REFERENCE_IMAGE_ACCENT_COLORS/);
  assert.match(componentSource, /progressColor=\{cardAccentColor\}/);
  assert.doesNotMatch(cssSource, /\.ant-/);
});

test('参数卡按栏内可用宽度自动决定列数并自然换行', async () => {
  const componentSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');
  const cssSource = await readFile(new URL('./media-parameter-panels.css', import.meta.url), 'utf8');

  assert.match(componentSource, /cardPadding=\{token\.paddingXXS\}/);
  assert.match(
    cssSource,
    /\.media-parameter-panels\s*\{[^}]*width:\s*100%[^}]*min-width:\s*0[^}]*container-type:\s*inline-size/s,
  );
  assert.doesNotMatch(
    cssSource,
    /\.media-parameter-panels\s*\{[^}]*(?:max-width|margin-inline):/s,
  );
  assert.match(
    cssSource,
    /\.media-parameter-panels__lane\s*\{[^}]*width:\s*100%[^}]*min-width:\s*0/s,
  );
  assert.doesNotMatch(cssSource, /\.media-parameter-panels__lane\s*\{[^}]*max-width:/s);
  assert.doesNotMatch(cssSource, /@container\s*\(max-width:\s*760px\)/);
  assert.match(
    cssSource,
    /@container\s*\(max-width:\s*560px\)[\s\S]*?\.media-parameter-panels__columns\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/s,
  );
  assert.match(
    cssSource,
    /\.media-parameter-section__grid\s*\{[^}]*grid-template-columns:\s*repeat\(auto-fit,\s*minmax\(min\(200px,\s*100%\),\s*1fr\)\)[^}]*align-items:\s*start[^}]*gap:\s*4px/s,
  );
  assert.match(cssSource, /\.media-parameter-card\s*\{[^}]*width:\s*100%/s);
  assert.doesNotMatch(cssSource, /\.media-parameter-card\s*\{[^}]*height:\s*100%/s);
  assert.match(cssSource, /\.media-parameter-card__value-area\s*\{[^}]*margin-top:\s*3px/s);
  assert.match(cssSource, /\.media-parameter-card__numeric-value\s*\{[^}]*gap:\s*2px/s);
  assert.match(
    cssSource,
    /\.media-parameter-card__bands\s*\{[^}]*grid-template-columns:\s*repeat\(3,\s*minmax\(0,\s*1fr\)\)/s,
  );
});

test('参数卡隐藏底部说明并使用不裁剪内容的固定高度', async () => {
  const componentSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');
  const cssSource = await readFile(new URL('./media-parameter-panels.css', import.meta.url), 'utf8');

  for (const removedDescriptionPath of [
    /\bdescriptionId\b/,
    /\brangeText\b/,
    /definition\.description/,
    /media-parameter-card__description/,
  ]) {
    assert.doesNotMatch(componentSource, removedDescriptionPath);
  }
  assert.doesNotMatch(cssSource, /media-parameter-card__description/);
  assert.doesNotMatch(cssSource, /overflow:\s*hidden/);
  assert.match(cssSource, /\.media-parameter-card\s*\{[^}]*height:\s*80px/s);
  assert.match(componentSource, /definition\.kind\s*===\s*'band-weights'[\s\S]*?media-parameter-card--band-weights/);
  assert.match(
    cssSource,
    /\.media-parameter-card--band-weights\s*\{[^}]*grid-column:\s*1\s*\/\s*-1[^}]*height:\s*196px/s,
  );
  assert.match(cssSource, /\.media-parameter-card__band-label\s*\{[^}]*flex-wrap:\s*wrap/s);
});

test('每张参数卡挂载时独立配色且顺序相邻颜色不重复', async () => {
  const feedback = await importTypeScriptModule('./parameter-panel-feedback.ts');
  const componentSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');

  const paths = Array.from({ length: 12 }, (_, index) => `video.field_${index}`);
  const assignments = feedback.createRandomParameterAccents(paths, () => 0);
  const accents = paths.map((path) => assignments[path]);

  assert.equal(Object.keys(assignments).length, paths.length);
  assert.deepEqual([...new Set(accents.slice(0, 5))].sort(), [...feedback.PARAMETER_ACCENT_KEYS].sort());
  assert.deepEqual([...new Set(accents.slice(5, 10))].sort(), [...feedback.PARAMETER_ACCENT_KEYS].sort());
  assert.ok(accents.some((accent, index) => index >= 5 && accents.slice(0, index).includes(accent)));
  assert.ok(accents.every((accent, index) => index === 0 || accent !== accents[index - 1]));
  assert.ok(accents.every((accent) => (
    ['cyan', 'orange', 'purple', 'magenta', 'yellow'].includes(accent)
  )));
  for (const randomValue of [0, 0.5, 0.999999]) {
    const boundaryAssignments = feedback.createRandomParameterAccents(paths, () => randomValue);
    const boundaryAccents = paths.map((path) => boundaryAssignments[path]);
    assert.ok(boundaryAccents.every((accent, index) => (
      index === 0 || accent !== boundaryAccents[index - 1]
    )));
  }
  assert.match(componentSource, /useState\(\(\)\s*=>\s*createRandomParameterAccents\(/);
  assert.match(componentSource, /parameterAccents\[path\]/);
  assert.match(componentSource, /key=\{path\}/);
  assert.doesNotMatch(componentSource, /key=\{index\}/);
  assert.match(componentSource, /progressColor=\{cardAccentColor\}/);
  assert.doesNotMatch(componentSource, /sectionAccents|sectionAccent|createRandomSectionAccents/);
});

test('每张参数卡都显示能力三态，面板顶部保留可见图例', async () => {
  const componentSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');

  assert.match(componentSource, /aria-label="参数接入状态说明"/);
  assert.match(componentSource, /Object\.keys\(MEDIA_PARAMETER_STATUS_LABELS\)/);
  assert.match(componentSource, /MEDIA_PARAMETER_STATUS_LABELS\[status\]/);
  assert.match(componentSource, /status=\{statusOverrides\?\.\[path\]\s*\?\?\s*definition\.status\}/);
});

test('参数值签名忽略对象键顺序并只在实际值变化时改变', async () => {
  const feedback = await importTypeScriptModule('./parameter-panel-feedback.ts');

  assert.equal(
    feedback.createMediaParameterValueSignature({ 65: 1, 92: 0.75 }),
    feedback.createMediaParameterValueSignature({ 92: 0.75, 65: 1 }),
  );
  assert.notEqual(
    feedback.createMediaParameterValueSignature({ 65: 1, 92: 0.75 }),
    feedback.createMediaParameterValueSignature({ 65: 1, 92: 0.8 }),
  );
  assert.equal(
    feedback.createMediaParameterValueSignature(null),
    feedback.createMediaParameterValueSignature(null),
  );
  assert.notEqual(
    feedback.createMediaParameterValueSignature(0),
    feedback.createMediaParameterValueSignature(-0),
  );
});

test('参数卡只在值变化时按自己的颜色执行可重复重启的缓慢淡出反馈', async () => {
  const componentSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');
  const cssSource = await readFile(new URL('./media-parameter-panels.css', import.meta.url), 'utf8');

  assert.match(componentSource, /createMediaParameterValueSignature\(rawValue\)/);
  assert.match(componentSource, /useState\(0\)/);
  assert.match(componentSource, /previousValueSignature\.current\s*===\s*valueSignature/);
  assert.match(componentSource, /setFlashGeneration\(\(current\)\s*=>\s*current\s*\+\s*1\)/);
  assert.match(componentSource, /flashGeneration\s*===\s*0\s*\?\s*baseCardClassName/);
  assert.match(componentSource, /media-parameter-card--flash-a/);
  assert.match(componentSource, /media-parameter-card--flash-b/);
  assert.match(componentSource, /'--media-parameter-accent':\s*progressColor/);
  assert.match(
    cssSource,
    /\.media-parameter-card::after\s*\{[^}]*border:[^}]*var\(--media-parameter-accent\)[^}]*box-shadow:[^}]*var\(--media-parameter-accent\)[^}]*opacity:\s*0[^}]*pointer-events:\s*none/s,
  );
  assert.match(
    cssSource,
    /\.media-parameter-card--flash-a::after[\s\S]*?\.media-parameter-card--flash-b::after\s*\{[^}]*animation-duration:\s*900ms[^}]*animation-timing-function:\s*ease-in-out/s,
  );
  for (const animationName of ['media-parameter-card-flash-a', 'media-parameter-card-flash-b']) {
    const animationStart = cssSource.indexOf(`@keyframes ${animationName}`);
    const animationEnd = cssSource.indexOf('\n}', animationStart);
    const animation = cssSource.slice(animationStart, animationEnd);
    assert.ok(animationStart >= 0 && animationEnd > animationStart);
    assert.match(animation, /10%\s*\{[^}]*opacity:\s*1/s);
    assert.match(animation, /100%\s*\{[^}]*opacity:\s*0/s);
    assert.doesNotMatch(animation, /box-shadow|border/);
  }
  assert.match(
    cssSource,
    /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.media-parameter-card--flash-a::after[\s\S]*?animation:\s*none/s,
  );
});

test('全部正式参数已接入，同时主参数面板保持只读职责', async () => {
  const definitionsSource = await readFile(new URL('./parameter-definitions.ts', import.meta.url), 'utf8');
  const panelSource = await readFile(new URL('./MediaParameterPanels.tsx', import.meta.url), 'utf8');

  assert.doesNotMatch(definitionsSource, /status: 'planned'/);
  assert.doesNotMatch(panelSource, /InputNumber|Slider|Switch|onChange/);
});
