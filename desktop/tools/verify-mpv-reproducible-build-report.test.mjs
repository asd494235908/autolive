import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { completeBuildLockFixture } from './mpv-reproducible-build-test-fixtures.mjs';
import {
  runReproducibleBuildReportGate,
  verifyReproducibleBuildReportMetadata,
} from './verify-mpv-reproducible-build-report.mjs';

const lockUrl = new URL('../third_party/mpv/reproducible-build-lock.json', import.meta.url);
const CONTRACT = Object.freeze({
  'build-log': ['build-log.txt', 'text'],
  'build-parameters': ['build-parameters.json', 'json'],
  'corresponding-source-archive': ['corresponding-source.tar.zst', 'tar-zst'],
  'copyright-inventory': ['copyright-inventory.txt', 'text'],
  'cyclonedx-sbom': ['sbom.cdx.json', 'cyclonedx-json'],
  'dependency-lock': ['dependency-lock.json', 'json'],
  'docker-host-evidence': ['docker-host-evidence.json', 'json'],
  'license-inventory': ['license-inventory.txt', 'text'],
  'meson-introspection': ['meson-introspection.json', 'json'],
  'mpv-executable': ['mpv.exe', 'pe'],
  'patch-bundle': ['patch-bundle.tar.zst', 'tar-zst'],
  'pe-imports': ['pe-imports.json', 'json'],
  'spdx-sbom': ['sbom.spdx.json', 'spdx-json'],
  'spirv-cross-runtime': ['spirv-cross-c-shared.dll', 'pe'],
  'vulkan-loader-runtime': ['vulkan-1.dll', 'pe'],
});

function digest(value) {
  return createHash('sha256').update(value).digest('hex');
}

async function reportFixture() {
  const lock = completeBuildLockFixture(JSON.parse(await readFile(lockUrl, 'utf8')));
  const lockBytes = Buffer.from(`${JSON.stringify(lock, null, 2)}\n`);
  const artifactContents = Object.fromEntries(Object.values(CONTRACT).map(([path]) => [path, Buffer.from(`${path}\n`)]));
  const report = {
    schema_version: 2,
    scope: 'phase7a_supply_candidate',
    claim: 'one_locked_cold_build_candidate',
    lock_sha256: digest(lockBytes),
    configured_inputs: {
      meson_arguments: [...lock.build_recipe.meson_arguments],
      spirv_cross_cmake_arguments: [...lock.build_recipe.spirv_cross_cmake_arguments],
      ffmpeg_arguments: [...lock.build_recipe.ffmpeg_arguments],
      cache_inventory: [...lock.cache_inventory],
    },
    artifacts: lock.output_policy.required_roles.map((role) => {
      const [path, format] = CONTRACT[role];
      return { role, path, size: artifactContents[path].length, sha256: digest(artifactContents[path]), format };
    }),
    dynamic_dependencies: Object.keys(lock.output_policy.bundled_dynamic_dependencies),
    build_environment: {
      target: lock.target,
      builder_image: lock.toolchain.builder_image,
      network: lock.fetch_policy.build_network,
      source_mode: lock.fetch_policy.source_mode,
      source_date_epoch: lock.build_recipe.environment.source_date_epoch,
      locale: lock.build_recipe.environment.locale,
      timezone: lock.build_recipe.environment.timezone,
      path_prefix_map: lock.build_recipe.environment.path_prefix_map,
    },
  };
  return { lock, lockBytes, report, artifactContents };
}

async function materializeFixture() {
  const root = await mkdtemp(join(tmpdir(), 'autolive-mpv-report-'));
  const fixture = await reportFixture();
  const lockPath = join(root, 'lock.json');
  const reportPath = join(root, 'report.json');
  const evidenceRoot = join(root, 'evidence');
  await mkdir(evidenceRoot);
  await writeFile(lockPath, fixture.lockBytes);
  await writeFile(reportPath, JSON.stringify(fixture.report, null, 2));
  for (const [path, content] of Object.entries(fixture.artifactContents)) await writeFile(join(evidenceRoot, path), content);
  return { root, lockPath, reportPath, evidenceRoot, ...fixture };
}

const semanticPass = async () => ({ semanticEvidenceVerified: true, claim: 'one_locked_cold_build_candidate' });

test('完整报告逐文件匹配后仍必须经过独立语义 verifier 才准入', async () => {
  const fixture = await materializeFixture();
  try {
    let called = 0;
    const report = await runReproducibleBuildReportGate({
      lockFilePath: fixture.lockPath,
      reportFilePath: fixture.reportPath,
      evidenceRootPath: fixture.evidenceRoot,
      buildInputsRootPath: fixture.root,
      semanticVerifier: async (options) => { called += 1; assert.equal(options.lockBytes.equals(fixture.lockBytes), true); return semanticPass(); },
    });
    assert.equal(called, 1);
    assert.equal(report.status, 'phase7a_supply_candidate');
    assert.equal(report.admitted, true);
    assert.equal(report.fileEvidenceVerified, true);
    assert.equal(report.semanticEvidenceVerified, true);
    assert.equal(report.artifactCount, 15);
    assert.deepEqual(report.limitations, ['不代表 bit-for-bit 可复现', '不代表 Phase 7B 法律/发布准入', '不代表 1080p 实机验收']);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('报告不能漂移锁、一次冷构建声明、动态依赖、角色固定路径或格式', async () => {
  const cases = [
    [(value) => { value.report.lock_sha256 = '0'.repeat(64); }, /lock_sha256/],
    [(value) => { value.report.claim = 'reproducible'; }, /claim/],
    [(value) => { value.report.configured_inputs.meson_arguments.pop(); }, /meson_arguments/],
    [(value) => { value.report.dynamic_dependencies.push('UNDECLARED.DLL'); }, /dynamic_dependencies/],
    [(value) => { value.report.artifacts.pop(); }, /artifacts roles/],
    [(value) => { value.report.artifacts[0].path = 'renamed.txt'; }, /路径与角色/],
    [(value) => { value.report.artifacts[0].format = 'binary'; }, /format/],
    [(value) => { value.report.admitted = true; }, /字段必须精确匹配/],
  ];
  for (const [mutate, pattern] of cases) {
    const fixture = await reportFixture();
    mutate(fixture);
    assert.throws(() => verifyReproducibleBuildReportMetadata(fixture), pattern);
  }
});

test('证据篡改、夹带以及语义 verifier 失败都阻断', async () => {
  const fixture = await materializeFixture();
  try {
    await writeFile(join(fixture.evidenceRoot, fixture.report.artifacts[0].path), 'tampered');
    await assert.rejects(runReproducibleBuildReportGate({
      lockFilePath: fixture.lockPath, reportFilePath: fixture.reportPath,
      evidenceRootPath: fixture.evidenceRoot, semanticVerifier: semanticPass,
    }), /大小不匹配|SHA-256 不匹配/);
  } finally { await rm(fixture.root, { recursive: true, force: true }); }

  const extra = await materializeFixture();
  try {
    await writeFile(join(extra.evidenceRoot, 'undeclared.txt'), 'extra');
    await assert.rejects(runReproducibleBuildReportGate({
      lockFilePath: extra.lockPath, reportFilePath: extra.reportPath,
      evidenceRootPath: extra.evidenceRoot, semanticVerifier: semanticPass,
    }), /不完全一致/);
  } finally { await rm(extra.root, { recursive: true, force: true }); }

  const semantic = await materializeFixture();
  try {
    await assert.rejects(runReproducibleBuildReportGate({
      lockFilePath: semantic.lockPath, reportFilePath: semantic.reportPath,
      evidenceRootPath: semantic.evidenceRoot,
      semanticVerifier: async () => { throw new Error('semantic mismatch'); },
    }), /semantic mismatch/);
  } finally { await rm(semantic.root, { recursive: true, force: true }); }

  for (const [result, pattern] of [
    [{ semanticEvidenceVerified: false, claim: 'one_locked_cold_build_candidate' }, /未明确验证通过/],
    [{ semanticEvidenceVerified: true, claim: 'reproducible' }, /claim 无效/],
  ]) {
    const rejected = await materializeFixture();
    try {
      await assert.rejects(runReproducibleBuildReportGate({
        lockFilePath: rejected.lockPath, reportFilePath: rejected.reportPath,
        evidenceRootPath: rejected.evidenceRoot,
        semanticVerifier: async () => result,
      }), pattern);
    } finally { await rm(rejected.root, { recursive: true, force: true }); }
  }
});

test('Phase 7A 报告测试与配方/归档语义测试进入桌面全量测试入口', async () => {
  const packageJson = JSON.parse(await readFile(new URL('../ui/package.json', import.meta.url), 'utf8'));
  for (const name of [
    'mpv-phase7a-build-recipe.test.mjs',
    'mpv-phase7a-publish.test.mjs',
    'verify-mpv-reproducible-build-evidence.test.mjs',
    'verify-mpv-reproducible-build-report.test.mjs',
  ]) assert.match(packageJson.scripts.test, new RegExp(name.replaceAll('.', '\\.')));
});
