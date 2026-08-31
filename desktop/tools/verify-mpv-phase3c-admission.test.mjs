import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import {
  HISTORY_FIELD_PATHS,
  buildPhase3cAdmissionReport,
  runPhase3cAdmissionGate,
} from './verify-mpv-phase3c-admission.mjs';

async function repositoryFixture() {
  const [manifestSource, cargoSource, rustCapabilitySource, backendSource,
    productionShaderSource, candidateShaderSource] = await Promise.all([
    readFile(new URL('../third_party/mpv/x86_64-pc-windows-msvc/legal/mpv-runtime-manifest.json', import.meta.url), 'utf8'),
    readFile(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8'),
    readFile(new URL('../src-tauri/src/media_video_gpu_effects.rs', import.meta.url), 'utf8'),
    readFile(new URL('../src-tauri/src/realtime_video_backend.rs', import.meta.url), 'utf8'),
    readFile(new URL('../src-tauri/resources/shaders/gpu83.hook', import.meta.url), 'utf8'),
    readFile(new URL('./shader-candidates/gpu83-baseline-candidate.hook', import.meta.url), 'utf8'),
  ]);
  return {
    manifest: JSON.parse(manifestSource),
    cargoSource,
    rustCapabilitySource,
    backendSource,
    productionShaderSource,
    candidateShaders: [{ path: 'baseline.hook', source: candidateShaderSource }],
    resourcePaths: ['runtime/mpv.exe'],
  };
}

test('Phase 3C 仓库门禁在 79/83 开发接线后仍拒绝颜色与两个历史字段', async () => {
  const report = await runPhase3cAdmissionGate();
  assert.equal(report.status, 'not_admitted');
  assert.equal(report.checkStatus, 'passed');
  assert.equal(report.admissionStatus, 'not_admitted');
  assert.equal(report.phase3cImplemented, false);
  assert.equal(report.productionCapability, '79/83');
  assert.equal(report.shaderCapability, '61/83');
  assert.equal(report.schedulerCapability, '18/83');
  assert.equal(report.unverifiedColorCapability, '2/83');
  assert.equal(report.currentTransport, 'external_mpv_json_ipc');
  assert.equal(report.historyPromotionAllowed, false);
  assert.equal(report.historyCapability.status, 'not_admitted');
  assert.deepEqual(report.historyCapability.fields, HISTORY_FIELD_PATHS);
  assert.deepEqual(report.crossFrameCombinationGuard, {
    status: 'not_admitted',
    field: 'advanced.picture_in_picture_timeline_locked',
    currentCapability: 'HISTORY',
    rule: '时间轴解锁不得仅因调度器可用而放行；还必须具备独立源时间轴与有界历史纹理证据',
  });
  assert.equal(report.productionIdentity.availableShaderOptions.length, 61);
  assert.deepEqual(report.reviewedPinnedSourceContract.publicBackends, ['opengl', 'sw']);
  assert.equal(report.reviewedPinnedSourceContract.verification,
    'external_primary_source_review_not_local_header_validation');
  assert.equal(report.reviewedPinnedSourceContract.d3d11PublicRenderBackend, false);
  assert.equal(report.reviewedPinnedSourceContract.vulkanPublicRenderBackend, false);
});

test('任一 HISTORY 字段被直接提升都会失败关闭', async () => {
  for (const field of HISTORY_FIELD_PATHS) {
    const fixture = await repositoryFixture();
    const escaped = field.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    fixture.rustCapabilitySource = fixture.rustCapabilitySource.replace(
      new RegExp(`((?:advanced_value|advanced_flag)!\\(\\s*"${escaped}"[\\s\\S]*?)HISTORY`),
      '$1AVAILABLE',
    );
    assert.throws(() => buildPhase3cAdmissionReport(fixture),
      /历史纹理字段集合|时间轴锁定字段|生产 shader 与 AVAILABLE 身份|生产 capability/, field);
  }
});

test('画中画时间轴锁定字段不能脱离跨帧组合门禁单独提升', async () => {
  const fixture = await repositoryFixture();
  fixture.rustCapabilitySource = fixture.rustCapabilitySource.replace(
    /(advanced_flag!\(\s*"advanced\.picture_in_picture_timeline_locked"[\s\S]*?)HISTORY/,
    '$1AVAILABLE',
  );
  assert.throws(() => buildPhase3cAdmissionReport(fixture), /时间轴锁定字段/);
});

test('未审核 libmpv 资产或 Cargo 依赖不能静默进入仓库', async () => {
  for (const artifact of ['runtime/mpv.dll', 'runtime/mpv-1.dll', 'runtime/libmpv-2.dll']) {
    const fixture = await repositoryFixture();
    fixture.resourcePaths.push(artifact);
    assert.throws(() => buildPhase3cAdmissionReport(fixture), /未审核的 libmpv/, artifact);
  }
  for (const dependency of [
    'libmpv-sys = "0.1"',
    '"libmpv-sys" = "0.1"',
    'renderer = { package = "libmpv-sys", version = "0.1" }',
    '[dependencies.renderer]\npackage = "libmpv-sys"\nversion = "0.1"',
  ]) {
    const fixture = await repositoryFixture();
    fixture.cargoSource += `\n${dependency}\n`;
    assert.throws(() => buildPhase3cAdmissionReport(fixture), /libmpv 依赖/, dependency);
  }
  const declarationFixture = await repositoryFixture();
  declarationFixture.resourceDeclarationSource = '{"relative_path":"binaries/mpv-2.dll"}';
  assert.throws(() => buildPhase3cAdmissionReport(declarationFixture), /运行资源.*libmpv/);
});

test('capability 常量、映射宏和生产 61 项身份都按真实语义锁定', async () => {
  const schedulerFixture = await repositoryFixture();
  schedulerFixture.rustCapabilitySource = schedulerFixture.rustCapabilitySource.replace(
    /const SCHEDULER:[\s\S]*?Gpu83ParameterCapability::ScheduledParameter;/,
    `// const SCHEDULER: Gpu83ParameterCapability = Gpu83ParameterCapability::ScheduledParameter;
const SCHEDULER: Gpu83ParameterCapability = Gpu83ParameterCapability::ShaderParameter;`,
  );
  assert.throws(() => buildPhase3cAdmissionReport(schedulerFixture), /SCHEDULER 不再由受限 PTS/);

  const schedulerStringFixture = await repositoryFixture();
  schedulerStringFixture.rustCapabilitySource = schedulerStringFixture.rustCapabilitySource.replace(
    /const SCHEDULER:[\s\S]*?Gpu83ParameterCapability::ScheduledParameter;/,
    `const SPOOF: &str = r#"const SCHEDULER: Gpu83ParameterCapability =
Gpu83ParameterCapability::ScheduledParameter;"#;
const SCHEDULER: Gpu83ParameterCapability = Gpu83ParameterCapability::ShaderParameter;`,
  );
  assert.throws(() => buildPhase3cAdmissionReport(schedulerStringFixture), /SCHEDULER 不再由受限 PTS/);

  const macroFixture = await repositoryFixture();
  macroFixture.rustCapabilitySource = macroFixture.rustCapabilitySource.replace(
    'capability: $capability,', 'capability: AVAILABLE, // capability: $capability',
  );
  assert.throws(() => buildPhase3cAdmissionReport(macroFixture), /video_value.*capability/);

  const macroStringFixture = await repositoryFixture();
  macroStringFixture.rustCapabilitySource = macroStringFixture.rustCapabilitySource.replace(
    'capability: $capability,', 'capability: AVAILABLE,',
  );
  macroStringFixture.rustCapabilitySource = `const SPOOF: &str =
"macro_rules! video_value { capability: $capability }";\n${macroStringFixture.rustCapabilitySource}`;
  assert.throws(() => buildPhase3cAdmissionReport(macroStringFixture), /video_value.*capability/);

  const visualBandFixture = await repositoryFixture();
  visualBandFixture.rustCapabilitySource = visualBandFixture.rustCapabilitySource.replace(
    'capability: AVAILABLE,',
    'capability: UNVERIFIED,',
  );
  assert.throws(() => buildPhase3cAdmissionReport(visualBandFixture), /visual_band 不再是已验证生产参数/);

  const identityFixture = await repositoryFixture();
  identityFixture.rustCapabilitySource = identityFixture.rustCapabilitySource
    .replace(/(video_value!\(\s*"video\.brightness_percent"[\s\S]*?)AVAILABLE/, '$1UNVERIFIED')
    .replace(/(video_value!\(\s*"video\.color_space_conversion_strength_percent"[\s\S]*?)UNVERIFIED/, '$1AVAILABLE');
  assert.throws(() => buildPhase3cAdmissionReport(identityFixture), /生产 shader 与 AVAILABLE 身份/);
});

test('unsafe 边界变化必须先触发专项审核', async () => {
  const fixture = await repositoryFixture();
  fixture.cargoSource = fixture.cargoSource.replace('unsafe_code = "forbid"', 'unsafe_code = "deny"');
  fixture.dependencyCargoSource = `${fixture.cargoSource}\n[lints.rust]\nunsafe_code = "forbid"\n`;
  assert.throws(() => buildPhase3cAdmissionReport(fixture), /unsafe_code/);

  for (const invalidSection of ['package.metadata.phase3c', 'workspace.lints.rust']) {
    const sectionFixture = await repositoryFixture();
    sectionFixture.cargoSource = sectionFixture.cargoSource.replace('[lints.rust]', `[${invalidSection}]`);
    assert.throws(() => buildPhase3cAdmissionReport(sectionFixture), /\[lints\.rust\]/,
      invalidSection);
  }
});

test('同帧 SAVE/BUFFER 可以存在但不构成跨帧准入证据', async () => {
  const fixture = await repositoryFixture();
  fixture.candidateShaders[0].source += '\n//!SAVE SAME_FRAME_INTERMEDIATE\n//!BUFFER SCRATCH\n';
  const report = buildPhase3cAdmissionReport(fixture);
  assert.equal(report.checkStatus, 'passed');
  assert.equal(report.admissionStatus, 'not_admitted');
  assert.equal(report.sameFrameShaderState.directiveCount, 2);
  assert.equal(report.sameFrameShaderState.admissionMeaning, 'none');
});

test('固定资产或外部 JSON IPC 契约漂移必须重新准入', async () => {
  const manifestFixture = await repositoryFixture();
  manifestFixture.manifest.components.mpv.source_ref = 'different';
  assert.throws(() => buildPhase3cAdmissionReport(manifestFixture), /源码引用漂移/);

  const transportFixture = await repositoryFixture();
  transportFixture.backendSource = transportFixture.backendSource.replace('--input-ipc-server=', '--removed-ipc=');
  assert.throws(() => buildPhase3cAdmissionReport(transportFixture), /JSON IPC/);
});

test('补齐法律材料不会被误判为 libmpv Render API 准入', async () => {
  const fixture = await repositoryFixture();
  fixture.manifest.files['legal/GPL-2.0.txt'] = { sha256: '0'.repeat(64) };
  fixture.resourcePaths.push('third_party/unrelated/client.h');
  const report = buildPhase3cAdmissionReport(fixture);
  assert.equal(report.checkStatus, 'passed');
  assert.ok(report.runtime.manifestFiles.includes('legal/GPL-2.0.txt'));
});

test('静态门禁自身不引入进程执行或真实媒体入口', async () => {
  const source = await readFile(new URL('./verify-mpv-phase3c-admission.mjs', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /node:child_process|\bspawn\s*\(|\bexecFile\s*\(/);
  assert.doesNotMatch(source, /confirm-real-media|ffmpeg\.exe/);
  assert.match(source, /tauri\.conf\.json/, '必须扫描 Tauri 打包资源配置');
  assert.doesNotMatch(source, /listFiles\(desktopRoot\)|listFiles\(join\(desktopRoot, 'third_party'\)\)/,
    '准入审计不得无界遍历整个 desktop 或无关 third_party');
});

test('Phase 3C 准入审计进入桌面默认静态测试入口', async () => {
  const packageJson = JSON.parse(await readFile(new URL('../ui/package.json', import.meta.url), 'utf8'));
  assert.match(packageJson.scripts.test, /verify-mpv-phase3c-admission\.test\.mjs/);
  assert.equal(packageJson.scripts['audit:mpv-phase3c-admission'],
    'node ../tools/verify-mpv-phase3c-admission.mjs');
});
