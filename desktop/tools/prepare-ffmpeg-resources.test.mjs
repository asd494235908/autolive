import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

import { applyBuildProfile, tauriBuildArguments, TEST_CONTROL_PLANE_BASE_URL } from './build-desktop-artifact.mjs';
import { readDesktopVersion } from './desktop-version.mjs';
import { prepareMediaRuntimeResources } from './prepare-ffmpeg-resources.mjs';

const script = fileURLToPath(new URL('./prepare-ffmpeg-resources.mjs', import.meta.url));
const archiveScript = fileURLToPath(new URL('./archive-desktop-artifact.mjs', import.meta.url));
const buildScript = fileURLToPath(new URL('./build-desktop-artifact.mjs', import.meta.url));

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

const legalFixture = Object.freeze({
  'Copyright.txt': 'mpv and libplacebo copyright notices',
  'GPL-2.0.txt': 'GPL-2.0-or-later license text',
  'LGPL-2.1.txt': 'LGPL-2.1-or-later license text',
  'SOURCE.md': 'source retrieval instructions',
  'THIRD-PARTY-NOTICES.md': 'audited dependency notices',
});

const windowsTarget = 'x86_64-pc-windows-msvc';
const runtimeFixture = Object.freeze({
  'mpv.exe': 'mpv-test',
  'spirv-cross-c-shared.dll': 'spirv-cross-test',
  'vulkan-1.dll': 'vulkan-test',
});
const componentFixture = Object.freeze({
  mpv: {
    source_ref: '7b8915bc1d04c7e1b61184e00c7fbfaab1911e75',
    license_expression: 'GPL-2.0-or-later',
  },
  libplacebo: {
    source_ref: '22ee762e8e0890fc54068beb670310f0edce7263',
    license_expression: 'LGPL-2.1-or-later',
  },
  ffmpeg: {
    source_ref: '1d7b14f61d66fdf18f15204c613df9d65396c319',
    license_expression: 'GPL-2.0-or-later',
  },
});
const evidenceFixture = Object.freeze({
  'corresponding-source.tar.zst': 'corresponding-source',
  'copyright-inventory.txt': 'copyright-inventory',
  'license-inventory.txt': 'license-inventory',
  'sbom.cdx.json': 'cyclonedx',
  'sbom.spdx.json': 'spdx',
});

function descriptor(content) {
  return { size_bytes: Buffer.byteLength(content), sha256: sha256(content) };
}

function writeJson(path, value) {
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function writePhase7bFixture(root, { approved = true } = {}) {
  const ffmpegRoot = join(root, 'ffmpeg');
  const mpvRoot = join(root, 'mpv');
  const targetRoot = join(mpvRoot, windowsTarget);
  const legalRoot = join(targetRoot, 'legal');
  const evidenceRoot = join(mpvRoot, 'build-evidence');
  mkdirSync(join(ffmpegRoot, windowsTarget), { recursive: true });
  mkdirSync(legalRoot, { recursive: true });
  mkdirSync(evidenceRoot, { recursive: true });
  writeFileSync(join(ffmpegRoot, windowsTarget, 'ffmpeg.exe'), 'ffmpeg-test');
  writeFileSync(join(ffmpegRoot, windowsTarget, 'ffprobe.exe'), 'ffprobe-test');

  for (const [name, content] of Object.entries(runtimeFixture)) {
    writeFileSync(join(targetRoot, name), content);
    writeFileSync(join(evidenceRoot, name), content);
  }
  for (const [name, content] of Object.entries(legalFixture)) {
    writeFileSync(join(legalRoot, name), content);
  }
  for (const [name, content] of Object.entries(evidenceFixture)) {
    writeFileSync(join(evidenceRoot, name), content);
  }

  const lock = {
    target: windowsTarget,
    sources: Object.entries(componentFixture).map(([name, value]) => ({
      kind: 'git', name, commit: value.source_ref, license_expression: value.license_expression,
    })),
  };
  const lockPath = join(mpvRoot, 'reproducible-build-lock.json');
  writeJson(lockPath, lock);
  const lockHash = sha256(readFileSync(lockPath));
  const artifacts = [
    ['mpv-executable', 'mpv.exe'],
    ['spirv-cross-runtime', 'spirv-cross-c-shared.dll'],
    ['vulkan-loader-runtime', 'vulkan-1.dll'],
    ['corresponding-source-archive', 'corresponding-source.tar.zst'],
    ['copyright-inventory', 'copyright-inventory.txt'],
    ['license-inventory', 'license-inventory.txt'],
    ['cyclonedx-sbom', 'sbom.cdx.json'],
    ['spdx-sbom', 'sbom.spdx.json'],
  ].map(([role, path]) => {
    const content = readFileSync(join(evidenceRoot, path));
    return { role, path, size: content.length, sha256: sha256(content) };
  });
  const report = {
    scope: 'phase7a_supply_candidate',
    claim: 'one_locked_cold_build_candidate',
    lock_sha256: lockHash,
    artifacts,
  };
  const reportPath = join(mpvRoot, 'reproducible-build-report.json');
  writeJson(reportPath, report);
  const reportDescriptor = descriptor(readFileSync(reportPath));
  const artifact = (role) => artifacts.find((entry) => entry.role === role);
  const artifactDescriptor = (role) => {
    const entry = artifact(role);
    return { size_bytes: entry.size, sha256: entry.sha256 };
  };
  const expectedIdentity = {
    target: windowsTarget,
    build: { lock_sha256: lockHash, report_sha256: reportDescriptor.sha256 },
    components: structuredClone(componentFixture),
    files: {
      'mpv.exe': artifactDescriptor('mpv-executable'),
      'spirv-cross-c-shared.dll': artifactDescriptor('spirv-cross-runtime'),
      'vulkan-1.dll': artifactDescriptor('vulkan-loader-runtime'),
      ...Object.fromEntries(Object.entries(legalFixture).map(([name, content]) => [
        `legal/${name}`, descriptor(content),
      ])),
    },
    supply_evidence: {
      report: { path: 'reproducible-build-report.json', ...reportDescriptor },
      corresponding_source: {
        path: 'build-evidence/corresponding-source.tar.zst',
        ...artifactDescriptor('corresponding-source-archive'),
      },
      copyright_inventory: {
        path: 'build-evidence/copyright-inventory.txt',
        ...artifactDescriptor('copyright-inventory'),
      },
      cyclonedx_sbom: {
        path: 'build-evidence/sbom.cdx.json', ...artifactDescriptor('cyclonedx-sbom'),
      },
      license_inventory: {
        path: 'build-evidence/license-inventory.txt', ...artifactDescriptor('license-inventory'),
      },
      spdx_sbom: {
        path: 'build-evidence/sbom.spdx.json', ...artifactDescriptor('spdx-sbom'),
      },
    },
  };
  const manifest = {
    schema_version: 2,
    target: windowsTarget,
    build: {
      scope: report.scope,
      claim: report.claim,
      ...expectedIdentity.build,
    },
    components: expectedIdentity.components,
    audit: approved
      ? {
          release_review_status: 'approved',
          corresponding_source_complete: true,
          third_party_notices_reviewed: true,
        }
      : {
          release_review_status: 'blocked',
          corresponding_source_complete: false,
          third_party_notices_reviewed: false,
        },
    files: expectedIdentity.files,
    supply_evidence: expectedIdentity.supply_evidence,
  };
  const manifestPath = join(legalRoot, 'mpv-runtime-manifest.json');
  writeJson(manifestPath, manifest);
  return { ffmpegRoot, mpvRoot, manifest, manifestPath, targetRoot };
}

function admittedSupplyGate() {
  return Promise.resolve({
    admitted: true,
    semanticEvidenceVerified: true,
    claim: 'one_locked_cold_build_candidate',
  });
}

function listFiles(root) {
  const files = [];
  function visit(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) visit(path);
      else files.push(relative(root, path).replaceAll('\\', '/'));
    }
  }
  visit(root);
  return files.sort();
}

test('缺少当前目标的 FFmpeg 文件时失败且不创建输出', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-ffmpeg-'));
  const source = join(root, 'source');
  const output = join(root, 'output');
  const target = 'aarch64-apple-darwin';
  const result = spawnSync(process.execPath, [script], {
    env: {
      ...process.env,
      AUTOLIVE_FFMPEG_SOURCE_DIR: source,
      AUTOLIVE_FFMPEG_OUTPUT_DIR: output,
      AUTOLIVE_TARGET_TRIPLE: target,
    },
    encoding: 'utf8',
  });

  assert.notEqual(result.status, 0);
  assert.match(`${result.stdout}${result.stderr}`, /缺少当前目标的 FFmpeg 资源/);
  assert.throws(() => statSync(output));
});

test('按目标三元组复制当前包所需的标准资源名', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-ffmpeg-'));
  const source = join(root, 'source', 'aarch64-apple-darwin');
  const output = join(root, 'output');
  const sourceDir = join(root, 'source');
  mkdirSync(source, { recursive: true });
  writeFileSync(join(source, 'ffmpeg'), 'ffmpeg-test');
  writeFileSync(join(source, 'ffprobe'), 'ffprobe-test');

  const result = spawnSync(process.execPath, [script], {
    env: {
      ...process.env,
      AUTOLIVE_FFMPEG_SOURCE_DIR: sourceDir,
      AUTOLIVE_FFMPEG_OUTPUT_DIR: output,
      AUTOLIVE_TARGET_TRIPLE: 'aarch64-apple-darwin',
    },
    encoding: 'utf8',
  });

  assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
  assert.equal(readFileSync(join(output, 'ffmpeg'), 'utf8'), 'ffmpeg-test');
  assert.equal(readFileSync(join(output, 'ffprobe'), 'utf8'), 'ffprobe-test');
});

test('Windows technical/blocked 清单不得晋级且失败保留旧输出', async () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-media-runtime-blocked-'));
  const fixture = writePhase7bFixture(root, { approved: false });
  const output = join(root, 'output');
  mkdirSync(output, { recursive: true });
  writeFileSync(join(output, 'old-runtime.txt'), 'keep-old');

  await assert.rejects(
    () => prepareMediaRuntimeResources({
      target: windowsTarget,
      ffmpegSourceRoot: fixture.ffmpegRoot,
      mpvSourceRoot: fixture.mpvRoot,
      outputRoot: output,
      supplyGate: admittedSupplyGate,
    }),
    /release.*approved/i,
  );

  assert.equal(readFileSync(join(output, 'old-runtime.txt'), 'utf8'), 'keep-old');
  assert.deepEqual(readdirSync(output), ['old-runtime.txt']);
});

test('Windows release 拒绝候选运行目录夹带文件且不污染旧输出', async () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-media-runtime-extra-'));
  const fixture = writePhase7bFixture(root);
  const output = join(root, 'output');
  mkdirSync(output, { recursive: true });
  writeFileSync(join(output, 'old-runtime.txt'), 'keep-old');
  writeFileSync(join(fixture.targetRoot, 'unreviewed.dll'), 'unreviewed');

  await assert.rejects(
    () => prepareMediaRuntimeResources({
      target: windowsTarget,
      ffmpegSourceRoot: fixture.ffmpegRoot,
      mpvSourceRoot: fixture.mpvRoot,
      outputRoot: output,
      supplyGate: admittedSupplyGate,
    }),
    /文件集|夹带|unreviewed/i,
  );

  assert.equal(readFileSync(join(output, 'old-runtime.txt'), 'utf8'), 'keep-old');
});

test('Windows release 原子生成固定十一项且不复制 Phase 7A 证据', async () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-media-runtime-release-'));
  const fixture = writePhase7bFixture(root);
  const output = join(root, 'output');
  mkdirSync(output, { recursive: true });
  writeFileSync(join(output, 'old-runtime.txt'), 'replace-old');

  await prepareMediaRuntimeResources({
    target: windowsTarget,
    ffmpegSourceRoot: fixture.ffmpegRoot,
    mpvSourceRoot: fixture.mpvRoot,
    outputRoot: output,
    supplyGate: admittedSupplyGate,
  });

  assert.deepEqual(listFiles(output), [
    'ffmpeg.exe',
    'ffprobe.exe',
    'licenses/mpv/Copyright.txt',
    'licenses/mpv/GPL-2.0.txt',
    'licenses/mpv/LGPL-2.1.txt',
    'licenses/mpv/SOURCE.md',
    'licenses/mpv/THIRD-PARTY-NOTICES.md',
    'licenses/mpv/mpv-runtime-manifest.json',
    'mpv.exe',
    'spirv-cross-c-shared.dll',
    'vulkan-1.dll',
  ]);
  assert.equal(existsSync(join(output, 'build-evidence')), false);
  assert.equal(existsSync(join(output, 'old-runtime.txt')), false);
  assert.deepEqual(
    JSON.parse(readFileSync(join(output, 'licenses/mpv/mpv-runtime-manifest.json'), 'utf8')),
    fixture.manifest,
  );
});


test('Tauri scripts use the package binary lookup that works on Windows', () => {
  const packageJsonPath = fileURLToPath(new URL('../ui/package.json', import.meta.url));
  const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
  const buildSource = readFileSync(buildScript, 'utf8');

  assert.match(packageJson.scripts['tauri:build'], /build-desktop-artifact\.mjs/);
  assert.match(packageJson.scripts['tauri:build:test'], /build-desktop-artifact\.mjs --profile=test/);
  assert.doesNotMatch(
    packageJson.scripts['tauri:build:test'],
    /prepare-ffmpeg-resources|generate-runtime-resource-tree/,
  );
  assert.match(buildSource, /'tauri'/);
  assert.deepEqual(tauriBuildArguments('x86_64-pc-windows-msvc'), [
    'build',
    '--config',
    'src-tauri/tauri.conf.json',
    '--bundles',
    'nsis',
  ]);
  assert.doesNotMatch(`${packageJson.scripts['tauri:dev']}\n${buildSource}`, /\.\/ui\/node_modules\/\.bin\/tauri/);
});

test('测试包固定使用测试控制面和测试 Tauri 配置', () => {
  const previous = {
    baseUrl: process.env.VITE_CONTROL_PLANE_BASE_URL,
    environment: process.env.VITE_CONTROL_PLANE_ENV,
    config: process.env.AUTOLIVE_TAURI_CONFIG,
    buildProfile: process.env.AUTOLIVE_BUILD_PROFILE,
    cargoTargetDir: process.env.CARGO_TARGET_DIR,
    bundleSourceDir: process.env.AUTOLIVE_BUNDLE_SOURCE_DIR,
    releaseExecutable: process.env.AUTOLIVE_RELEASE_EXECUTABLE,
    packageRoot: process.env.AUTOLIVE_PACKAGE_ROOT,
  };
  try {
    delete process.env.VITE_CONTROL_PLANE_BASE_URL;
    delete process.env.VITE_CONTROL_PLANE_ENV;
    delete process.env.AUTOLIVE_TAURI_CONFIG;
    delete process.env.AUTOLIVE_BUILD_PROFILE;
    delete process.env.CARGO_TARGET_DIR;
    delete process.env.AUTOLIVE_BUNDLE_SOURCE_DIR;
    delete process.env.AUTOLIVE_RELEASE_EXECUTABLE;
    delete process.env.AUTOLIVE_PACKAGE_ROOT;
    applyBuildProfile('test');
    assert.equal(TEST_CONTROL_PLANE_BASE_URL, 'http://101.96.208.132:9090');
    assert.equal(process.env.VITE_CONTROL_PLANE_BASE_URL, TEST_CONTROL_PLANE_BASE_URL);
    assert.equal(process.env.VITE_CONTROL_PLANE_ENV, 'test');
    assert.equal(process.env.AUTOLIVE_TAURI_CONFIG, 'src-tauri/tauri.test.conf.json');
    assert.equal(process.env.AUTOLIVE_BUILD_PROFILE, 'test');
    assert.match(process.env.CARGO_TARGET_DIR, /src-tauri[\\/]target-test-package$/);
    assert.match(process.env.AUTOLIVE_BUNDLE_SOURCE_DIR, /target-test-package[\\/]debug[\\/]bundle$/);
    assert.match(process.env.AUTOLIVE_RELEASE_EXECUTABLE, /target-test-package[\\/]debug[\\/]autolive-desktop-core\.exe$/);
    assert.match(process.env.AUTOLIVE_PACKAGE_ROOT, /desktop[\\/]package-test$/);
    assert.deepEqual(tauriBuildArguments('x86_64-pc-windows-msvc'), [
      'build',
      '--config',
      'src-tauri/tauri.test.conf.json',
      '--debug',
      '--no-sign',
      '--bundles',
      'nsis',
    ]);
  } finally {
    for (const [key, value] of Object.entries({
      VITE_CONTROL_PLANE_BASE_URL: previous.baseUrl,
      VITE_CONTROL_PLANE_ENV: previous.environment,
      AUTOLIVE_TAURI_CONFIG: previous.config,
      AUTOLIVE_BUILD_PROFILE: previous.buildProfile,
      CARGO_TARGET_DIR: previous.cargoTargetDir,
      AUTOLIVE_BUNDLE_SOURCE_DIR: previous.bundleSourceDir,
      AUTOLIVE_RELEASE_EXECUTABLE: previous.releaseExecutable,
      AUTOLIVE_PACKAGE_ROOT: previous.packageRoot,
    })) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
});

test('归档脚本按版本和目标平台目录保存 bundle', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-package-'));
  const source = join(root, 'bundle');
  const output = join(root, 'package');
  const { release } = readDesktopVersion();
  mkdirSync(join(source, 'dmg'), { recursive: true });
  mkdirSync(join(source, 'macos', 'autolive.app'), { recursive: true });
  writeFileSync(join(source, 'dmg', 'autolive.dmg'), 'bundle-test');
  writeFileSync(join(source, 'macos', 'autolive.app', 'Contents.txt'), 'not-an-installer');

  const result = spawnSync(process.execPath, [archiveScript], {
    env: {
      ...process.env,
      AUTOLIVE_BUNDLE_SOURCE_DIR: source,
      AUTOLIVE_PACKAGE_ROOT: output,
      AUTOLIVE_TARGET_TRIPLE: 'aarch64-apple-darwin',
    },
    encoding: 'utf8',
  });

  assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
  assert.equal(
    readFileSync(join(output, release, 'aarch64-apple-darwin', 'dmg', 'autolive.dmg'), 'utf8'),
    'bundle-test',
  );
  assert.throws(() => statSync(join(output, release, 'aarch64-apple-darwin', 'macos')));
});

test('Windows 正式包归档 NSIS EXE 和 Tauri portable', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-package-'));
  const executable = join(root, 'target', 'release', 'autolive-desktop-core.exe');
  const manifest = join(root, 'runtime-resources.json');
  const embedded = join(root, 'embedded-runtime-resources');
  const bundleSource = join(root, 'target', 'release', 'bundle');
  const output = join(root, 'package');
  const { release } = readDesktopVersion();
  mkdirSync(join(root, 'target', 'release'), { recursive: true });
  mkdirSync(join(bundleSource, 'nsis'), { recursive: true });
  mkdirSync(join(embedded, 'x86_64-pc-windows-msvc', 'binaries'), { recursive: true });
  writeFileSync(executable, 'app-test');
  writeFileSync(join(bundleSource, 'nsis', 'autolive-setup.exe'), 'setup-test');
  writeFileSync(join(embedded, 'x86_64-pc-windows-msvc', 'binaries', 'ffmpeg.exe'), 'ffmpeg');
  writeFileSync(manifest, JSON.stringify({
    schema_version: 1,
    release,
    target: 'x86_64-pc-windows-msvc',
    files: [{
      relative_path: 'x86_64-pc-windows-msvc/binaries/ffmpeg.exe',
      size_bytes: 6,
      sha256: sha256('ffmpeg'),
      component: 'media',
      executable: true,
    }],
  }));

  const result = spawnSync(process.execPath, [archiveScript], {
    env: {
      ...process.env,
      AUTOLIVE_RELEASE_EXECUTABLE: executable,
      AUTOLIVE_RUNTIME_RESOURCE_MANIFEST: manifest,
      AUTOLIVE_EMBEDDED_RESOURCE_DIR: embedded,
      AUTOLIVE_BUNDLE_SOURCE_DIR: bundleSource,
      AUTOLIVE_PACKAGE_ROOT: output,
      AUTOLIVE_TARGET_TRIPLE: 'x86_64-pc-windows-msvc',
      AUTOLIVE_SKIP_PORTABLE_ZIP: '1',
    },
    encoding: 'utf8',
  });

  assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
  const bundle = join(output, release, 'x86_64-pc-windows-msvc');
  assert.equal(readFileSync(join(bundle, 'portable', 'autolive-desktop-core.exe'), 'utf8'), 'app-test');
  assert.equal(readFileSync(join(bundle, 'nsis', 'autolive-setup.exe'), 'utf8'), 'setup-test');
  assert.throws(() => statSync(join(bundle, 'msi')));
});

test('GitHub workflow uploads the versioned package directory', () => {
  const workflowPath = fileURLToPath(
    new URL('../../.github/workflows/desktop-package.yml', import.meta.url),
  );
  const workflow = readFileSync(workflowPath, 'utf8');

  assert.match(workflow, /id: desktop-version/);
  assert.match(
    workflow,
    /path: desktop\/package\/\$\{\{ steps\.desktop-version\.outputs\.version \}\}\/\$\{\{ matrix\.target_triple \}\}\/\*\*/,
  );
  assert.match(
    workflow,
    /name: desktop-bundle-\$\{\{ steps\.desktop-version\.outputs\.version \}\}-\$\{\{ matrix\.target_triple \}\}/,
  );
});
