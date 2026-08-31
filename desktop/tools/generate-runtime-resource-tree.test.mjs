import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import test from 'node:test';

import {
  buildRuntimeResourceRelease,
  RESOURCE_RELEASE,
} from './generate-runtime-resource-tree.mjs';
import { readDesktopVersion } from './desktop-version.mjs';
import { validateMpvRuntimeManifestV2 } from './verify-mpv-runtime-manifest-v2.mjs';

const DEPLOY_INVENTORY = 'autolive-deploy-inventory.json';
const expectedRelease = readDesktopVersion().release;

function writeFixture(root, relativePath, content) {
  const path = join(root, ...relativePath.split('/'));
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, content);
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

const windowsTarget = 'x86_64-pc-windows-msvc';
const windowsRuntime = Object.freeze({
  'ffmpeg.exe': 'ffmpeg',
  'ffprobe.exe': 'ffprobe',
  'mpv.exe': 'mpv',
  'spirv-cross-c-shared.dll': 'spirv',
  'vulkan-1.dll': 'vulkan',
  'licenses/mpv/Copyright.txt': 'copyright',
  'licenses/mpv/GPL-2.0.txt': 'gpl',
  'licenses/mpv/LGPL-2.1.txt': 'lgpl',
  'licenses/mpv/SOURCE.md': 'source',
  'licenses/mpv/THIRD-PARTY-NOTICES.md': 'notices',
});

function descriptor(value) {
  return { size_bytes: Buffer.byteLength(value), sha256: sha256(value) };
}

function windowsManifestFixture({ approved = true } = {}) {
  const expectedIdentity = {
    target: windowsTarget,
    build: { lock_sha256: '1'.repeat(64), report_sha256: '2'.repeat(64) },
    components: {
      mpv: { source_ref: '3'.repeat(40), license_expression: 'GPL-2.0-or-later' },
      libplacebo: { source_ref: '4'.repeat(40), license_expression: 'LGPL-2.1-or-later' },
      ffmpeg: { source_ref: '5'.repeat(40), license_expression: 'GPL-2.0-or-later' },
    },
    files: {
      'mpv.exe': descriptor(windowsRuntime['mpv.exe']),
      'spirv-cross-c-shared.dll': descriptor(windowsRuntime['spirv-cross-c-shared.dll']),
      'vulkan-1.dll': descriptor(windowsRuntime['vulkan-1.dll']),
      ...Object.fromEntries([
        'Copyright.txt', 'GPL-2.0.txt', 'LGPL-2.1.txt', 'SOURCE.md', 'THIRD-PARTY-NOTICES.md',
      ].map((name) => [
        `legal/${name}`, descriptor(windowsRuntime[`licenses/mpv/${name}`]),
      ])),
    },
    supply_evidence: {
      report: { path: 'reproducible-build-report.json', size_bytes: 1, sha256: '2'.repeat(64) },
      corresponding_source: {
        path: 'build-evidence/corresponding-source.tar.zst', size_bytes: 1, sha256: '6'.repeat(64),
      },
      copyright_inventory: {
        path: 'build-evidence/copyright-inventory.txt', size_bytes: 1, sha256: '7'.repeat(64),
      },
      cyclonedx_sbom: {
        path: 'build-evidence/sbom.cdx.json', size_bytes: 1, sha256: '8'.repeat(64),
      },
      license_inventory: {
        path: 'build-evidence/license-inventory.txt', size_bytes: 1, sha256: '9'.repeat(64),
      },
      spdx_sbom: {
        path: 'build-evidence/sbom.spdx.json', size_bytes: 1, sha256: 'a'.repeat(64),
      },
    },
  };
  const manifest = {
    schema_version: 2,
    target: windowsTarget,
    build: {
      scope: 'phase7a_supply_candidate',
      claim: 'one_locked_cold_build_candidate',
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
  return { manifest, expectedIdentity };
}

function writeWindowsPreparedTree(sourceRoot, options) {
  const fixture = windowsManifestFixture(options);
  for (const [path, content] of Object.entries(windowsRuntime)) {
    writeFixture(sourceRoot, `binaries/${path}`, content);
  }
  writeFixture(
    sourceRoot,
    'binaries/licenses/mpv/mpv-runtime-manifest.json',
    `${JSON.stringify(fixture.manifest, null, 2)}\n`,
  );
  return fixture;
}

function releaseGate(expectedIdentity) {
  return async ({ binariesRoot, mode }) => {
    const manifestBytes = readFileSync(join(binariesRoot, 'licenses/mpv/mpv-runtime-manifest.json'));
    const manifest = JSON.parse(manifestBytes);
    validateMpvRuntimeManifestV2(manifest, { mode, expectedIdentity });
    return { manifest, expectedIdentity, manifestDescriptor: descriptor(manifestBytes) };
  };
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

test('只发布当前目标的 FFmpeg 媒体资源', async () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-runtime-resources-'));
  const sourceRoot = join(root, 'src-tauri');
  const outputRoot = join(root, 'output');
  const manifestPath = join(root, 'runtime-resources.json');
  const largeBinary = Buffer.alloc(64 * 1024 + 1, 0x5a);

  writeFixture(sourceRoot, 'binaries/ffmpeg', largeBinary);
  writeFixture(sourceRoot, 'binaries/ffprobe', 'ffprobe');
  writeFixture(sourceRoot, 'binaries/.gitignore', 'ignored');
  writeFixture(sourceRoot, 'binaries/.hidden/tool', 'ignored');
  writeFixture(sourceRoot, 'binaries/download.log', 'ignored');
  writeFixture(sourceRoot, 'binaries/trees/tree.json', 'ignored');
  mkdirSync(join(sourceRoot, 'binaries/empty'), { recursive: true });

  const result = await buildRuntimeResourceRelease({
    target: 'aarch64-apple-darwin',
    sourceRoot,
    outputRoot,
    manifestPath,
  });

  assert.deepEqual(result.manifest.files, [
    {
      relative_path: 'aarch64-apple-darwin/binaries/ffmpeg',
      sha256: sha256(largeBinary),
      size_bytes: largeBinary.length,
      executable: true,
      component: 'media',
    },
    {
      relative_path: 'aarch64-apple-darwin/binaries/ffprobe',
      sha256: sha256('ffprobe'),
      size_bytes: 7,
      executable: true,
      component: 'media',
    },
  ]);
  assert.equal(result.manifest.schema_version, 1);
  assert.equal(RESOURCE_RELEASE, expectedRelease);
  assert.equal(result.manifest.release, expectedRelease);
  assert.equal(result.manifest.target, 'aarch64-apple-darwin');
  assert.equal(
    result.manifest.base_url,
    `http://101.96.208.132:7088/autolive-resources/${expectedRelease}/`,
  );
  assert.deepEqual(JSON.parse(readFileSync(manifestPath, 'utf8')), result.manifest);
  assert.equal(
    readFileSync(join(result.embeddedRoot, 'aarch64-apple-darwin/binaries/ffmpeg')).length,
    largeBinary.length,
  );
  assert.equal(existsSync(join(result.embeddedRoot, 'common')), false);
  assert.equal(existsSync(join(result.releaseRoot, 'common')), false);
  assert.equal(existsSync(join(result.releaseRoot, 'aarch64-apple-darwin/binaries/.hidden')), false);
  assert.equal(existsSync(join(result.releaseRoot, 'aarch64-apple-darwin/binaries/download.log')), false);
  assert.equal(existsSync(join(sourceRoot, 'binaries/.hidden/tool')), true);
  assert.equal(existsSync(join(sourceRoot, 'binaries/download.log')), true);

  const inventory = JSON.parse(
    readFileSync(join(result.releaseRoot, 'aarch64-apple-darwin', DEPLOY_INVENTORY), 'utf8'),
  );
  assert.deepEqual(inventory, {
    schema: 1,
    release: expectedRelease,
    scope: 'aarch64-apple-darwin',
    files: [
      {
        relative_path: 'binaries/ffmpeg',
        type: 'regular-file',
        size_bytes: largeBinary.length,
        sha256: sha256(largeBinary),
      },
      {
        relative_path: 'binaries/ffprobe',
        type: 'regular-file',
        size_bytes: 7,
        sha256: sha256('ffprobe'),
      },
    ],
  });
});

test('发布清单与内嵌资源保留合法空文件', async () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-runtime-empty-file-'));
  const sourceRoot = join(root, 'src-tauri');
  const outputRoot = join(root, 'output');
  const manifestPath = join(root, 'runtime-resources.json');

  writeFixture(sourceRoot, 'binaries/ffmpeg', 'ffmpeg');
  writeFixture(sourceRoot, 'binaries/ffprobe', '');

  const result = await buildRuntimeResourceRelease({
    target: 'aarch64-apple-darwin',
    sourceRoot,
    outputRoot,
    manifestPath,
  });
  const relativePath = 'aarch64-apple-darwin/binaries/ffprobe';

  assert.deepEqual(
    result.manifest.files.find((file) => file.relative_path === relativePath),
    {
      relative_path: relativePath,
      sha256: sha256(''),
      size_bytes: 0,
      executable: true,
      component: 'media',
    },
  );
  assert.equal(readFileSync(join(result.embeddedRoot, ...relativePath.split('/'))).length, 0);
});

test('Windows blocked v2 不得生成发布树且保留三个旧输出', async () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-runtime-blocked-'));
  const sourceRoot = join(root, 'src-tauri');
  const outputRoot = join(root, 'output');
  const manifestPath = join(root, 'runtime-resources.json');
  const fixture = writeWindowsPreparedTree(sourceRoot, { approved: false });
  const oldRelease = join(outputRoot, 'autolive-resources', expectedRelease);
  const oldEmbedded = join(sourceRoot, 'embedded-runtime-resources');
  writeFixture(oldRelease, 'old-release.txt', 'keep-release');
  writeFixture(oldEmbedded, 'old-embedded.txt', 'keep-embedded');
  writeFileSync(manifestPath, 'keep-manifest');

  await assert.rejects(
    () => buildRuntimeResourceRelease({
      target: windowsTarget,
      sourceRoot,
      outputRoot,
      manifestPath,
      mpvReleaseGate: releaseGate(fixture.expectedIdentity),
    }),
    /release.*approved/i,
  );

  assert.equal(readFileSync(join(oldRelease, 'old-release.txt'), 'utf8'), 'keep-release');
  assert.equal(readFileSync(join(oldEmbedded, 'old-embedded.txt'), 'utf8'), 'keep-embedded');
  assert.equal(readFileSync(manifestPath, 'utf8'), 'keep-manifest');
});

test('Windows release 只生成固定十一项且供应证据不进入运行树', async () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-runtime-release-'));
  const sourceRoot = join(root, 'src-tauri');
  const outputRoot = join(root, 'output');
  const manifestPath = join(root, 'runtime-resources.json');
  const fixture = writeWindowsPreparedTree(sourceRoot);

  const result = await buildRuntimeResourceRelease({
    target: windowsTarget,
    sourceRoot,
    outputRoot,
    manifestPath,
    mpvReleaseGate: releaseGate(fixture.expectedIdentity),
  });

  const expectedFiles = [
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
  ];
  assert.deepEqual(listFiles(join(result.embeddedRoot, windowsTarget, 'binaries')), expectedFiles);
  assert.deepEqual(
    result.manifest.files.map((file) => file.relative_path),
    expectedFiles
      .map((path) => [windowsTarget, 'binaries', path].join('/'))
      .sort((left, right) => left.localeCompare(right)),
  );
  assert.equal(
    result.manifest.files.find((file) => file.relative_path.endsWith('/mpv.exe'))?.executable,
    true,
  );
  assert.equal(
    result.manifest.files.find((file) => file.relative_path.includes('/licenses/'))?.executable,
    false,
  );
  assert.equal(existsSync(join(result.embeddedRoot, 'build-evidence')), false);
  assert.equal(result.manifest.files.some((file) => file.relative_path.includes('build-evidence')), false);
});

test('Windows release 拒绝准入后被等价重写的内部清单', async () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-runtime-manifest-identity-'));
  const sourceRoot = join(root, 'src-tauri');
  const outputRoot = join(root, 'output');
  const manifestPath = join(root, 'runtime-resources.json');
  const fixture = writeWindowsPreparedTree(sourceRoot);
  const internalManifest = join(sourceRoot, 'binaries/licenses/mpv/mpv-runtime-manifest.json');

  await assert.rejects(
    () => buildRuntimeResourceRelease({
      target: windowsTarget,
      sourceRoot,
      outputRoot,
      manifestPath,
      mpvReleaseGate: async ({ mode }) => {
        const original = readFileSync(internalManifest);
        validateMpvRuntimeManifestV2(JSON.parse(original), {
          mode,
          expectedIdentity: fixture.expectedIdentity,
        });
        writeFileSync(internalManifest, JSON.stringify(fixture.manifest));
        return {
          manifest: fixture.manifest,
          expectedIdentity: fixture.expectedIdentity,
          manifestDescriptor: descriptor(original),
        };
      },
    }),
    /manifest.*(大小|SHA-256)/i,
  );
  assert.equal(existsSync(manifestPath), false);
});


test('拒绝未支持的目标三元组', async () => {
  await assert.rejects(
    () => buildRuntimeResourceRelease({ target: 'x86_64-unknown-linux-gnu' }),
    /当前构建平台不受支持/,
  );
});
