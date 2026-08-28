import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

import { applyBuildProfile, tauriBuildArguments, TEST_CONTROL_PLANE_BASE_URL } from './build-desktop-artifact.mjs';
import { readDesktopVersion } from './desktop-version.mjs';

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
  'D3DCOMPILER_43-EULA.txt': 'Microsoft redistributable license terms',
});

function fixedMpvManifest() {
  return {
    schema_version: 1,
    artifact: {
      build_repository: 'https://github.com/shinchiro/mpv-winbuild-cmake',
      release: '20260814',
      asset: 'mpv-x86_64-20260814-git-7b8915bc1d.7z',
      download_url: 'https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20260814/mpv-x86_64-20260814-git-7b8915bc1d.7z',
      archive_sha256: '1bf3b029da2c98e605e00e85f21ee3142f22a1dcc4ceb5c827b5c51e36e390f9',
    },
    components: {
      mpv: {
        version: 'v0.41.0-923-g7b8915bc1',
        built_at: 'Aug 14 2026 00:27:31',
        source_repository: 'https://github.com/mpv-player/mpv',
        source_ref: '7b8915bc1d',
        license_expression: 'GPL-2.0-or-later',
        copyright_file: 'Copyright.txt',
        license_file: 'GPL-2.0.txt',
        source_file: 'SOURCE.md',
      },
      libplacebo: {
        version: 'v7.371.0',
        build_revision: 'v7.360.0-111-g22ee762-dirty',
        source_repository: 'https://github.com/haasn/libplacebo',
        source_ref: '22ee762',
        license_expression: 'LGPL-2.1-or-later',
        license_file: 'LGPL-2.1.txt',
        source_file: 'SOURCE.md',
      },
      d3dcompiler_43: {
        license_expression: 'LicenseRef-Microsoft-DirectX-Redistributable-EULA',
        license_file: 'D3DCOMPILER_43-EULA.txt',
      },
    },
    files: {
      'mpv.exe': { sha256: sha256('mpv-test') },
      'd3dcompiler_43.dll': { sha256: sha256('d3dcompiler-test') },
      ...Object.fromEntries(
        Object.entries(legalFixture).map(([name, content]) => [
          `legal/${name}`,
          { sha256: sha256(content) },
        ]),
      ),
    },
  };
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

test('Windows 构建要求许可证材料并复制固定 mpv GPU 运行资源和清单', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-media-runtime-'));
  const target = 'x86_64-pc-windows-msvc';
  const ffmpegRoot = join(root, 'ffmpeg');
  const mpvRoot = join(root, 'mpv');
  const output = join(root, 'output');
  mkdirSync(join(ffmpegRoot, target), { recursive: true });
  mkdirSync(join(mpvRoot, target), { recursive: true });
  writeFileSync(join(ffmpegRoot, target, 'ffmpeg.exe'), 'ffmpeg-test');
  writeFileSync(join(ffmpegRoot, target, 'ffprobe.exe'), 'ffprobe-test');

  const missing = spawnSync(process.execPath, [script], {
    env: {
      ...process.env,
      AUTOLIVE_FFMPEG_SOURCE_DIR: ffmpegRoot,
      AUTOLIVE_MPV_SOURCE_DIR: mpvRoot,
      AUTOLIVE_FFMPEG_OUTPUT_DIR: output,
      AUTOLIVE_TARGET_TRIPLE: target,
    },
    encoding: 'utf8',
  });
  assert.notEqual(missing.status, 0);
  assert.match(`${missing.stdout}${missing.stderr}`, /缺少当前目标的 mpv/);

  writeFileSync(join(mpvRoot, target, 'mpv.exe'), 'mpv-test');
  writeFileSync(join(mpvRoot, target, 'd3dcompiler_43.dll'), 'd3dcompiler-test');
  const missingLegal = spawnSync(process.execPath, [script], {
    env: {
      ...process.env,
      AUTOLIVE_FFMPEG_SOURCE_DIR: ffmpegRoot,
      AUTOLIVE_MPV_SOURCE_DIR: mpvRoot,
      AUTOLIVE_FFMPEG_OUTPUT_DIR: output,
      AUTOLIVE_TARGET_TRIPLE: target,
    },
    encoding: 'utf8',
  });
  assert.notEqual(missingLegal.status, 0);
  assert.match(`${missingLegal.stdout}${missingLegal.stderr}`, /mpv 许可证/);
  assert.throws(() => statSync(output));

  const legalRoot = join(mpvRoot, target, 'legal');
  mkdirSync(legalRoot, { recursive: true });
  for (const [name, content] of Object.entries(legalFixture)) {
    writeFileSync(join(legalRoot, name), content);
  }
  const manifestPath = join(legalRoot, 'mpv-runtime-manifest.json');
  const invalidManifest = fixedMpvManifest();
  invalidManifest.components.libplacebo.license_expression = 'GPL-2.0-or-later';
  writeFileSync(manifestPath, `${JSON.stringify(invalidManifest, null, 2)}\n`);
  const invalid = spawnSync(process.execPath, [script], {
    env: {
      ...process.env,
      AUTOLIVE_FFMPEG_SOURCE_DIR: ffmpegRoot,
      AUTOLIVE_MPV_SOURCE_DIR: mpvRoot,
      AUTOLIVE_FFMPEG_OUTPUT_DIR: output,
      AUTOLIVE_TARGET_TRIPLE: target,
    },
    encoding: 'utf8',
  });
  assert.notEqual(invalid.status, 0);
  assert.match(
    `${invalid.stdout}${invalid.stderr}`,
    /mpv 发布清单字段不匹配：components\.libplacebo\.license_expression/,
  );
  assert.throws(() => statSync(output));

  writeFileSync(manifestPath, `${JSON.stringify(fixedMpvManifest(), null, 2)}\n`);
  writeFileSync(join(legalRoot, 'SOURCE.md'), 'tampered source instructions');
  const tampered = spawnSync(process.execPath, [script], {
    env: {
      ...process.env,
      AUTOLIVE_FFMPEG_SOURCE_DIR: ffmpegRoot,
      AUTOLIVE_MPV_SOURCE_DIR: mpvRoot,
      AUTOLIVE_FFMPEG_OUTPUT_DIR: output,
      AUTOLIVE_TARGET_TRIPLE: target,
    },
    encoding: 'utf8',
  });
  assert.notEqual(tampered.status, 0);
  assert.match(`${tampered.stdout}${tampered.stderr}`, /mpv 发布清单文件哈希不匹配：legal[\\/]SOURCE\.md/);
  assert.throws(() => statSync(output));

  writeFileSync(join(legalRoot, 'SOURCE.md'), legalFixture['SOURCE.md']);
  writeFileSync(manifestPath, `${JSON.stringify(fixedMpvManifest(), null, 2)}\n`);
  const prepared = spawnSync(process.execPath, [script], {
    env: {
      ...process.env,
      AUTOLIVE_FFMPEG_SOURCE_DIR: ffmpegRoot,
      AUTOLIVE_MPV_SOURCE_DIR: mpvRoot,
      AUTOLIVE_FFMPEG_OUTPUT_DIR: output,
      AUTOLIVE_TARGET_TRIPLE: target,
    },
    encoding: 'utf8',
  });
  assert.equal(prepared.status, 0, `${prepared.stdout}${prepared.stderr}`);
  assert.equal(readFileSync(join(output, 'mpv.exe'), 'utf8'), 'mpv-test');
  assert.equal(readFileSync(join(output, 'd3dcompiler_43.dll'), 'utf8'), 'd3dcompiler-test');
  const publishedLegalRoot = join(output, 'licenses', 'mpv');
  assert.equal(readFileSync(join(publishedLegalRoot, 'SOURCE.md'), 'utf8'), 'source retrieval instructions');
  assert.equal(
    readFileSync(join(publishedLegalRoot, 'D3DCOMPILER_43-EULA.txt'), 'utf8'),
    'Microsoft redistributable license terms',
  );
  assert.deepEqual(
    JSON.parse(readFileSync(join(publishedLegalRoot, 'mpv-runtime-manifest.json'), 'utf8')),
    fixedMpvManifest(),
  );
});

test('Tauri scripts use the package binary lookup that works on Windows', () => {
  const packageJsonPath = fileURLToPath(new URL('../ui/package.json', import.meta.url));
  const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
  const buildSource = readFileSync(buildScript, 'utf8');

  assert.match(packageJson.scripts['tauri:build'], /build-desktop-artifact\.mjs/);
  assert.match(packageJson.scripts['tauri:build:test'], /build-desktop-artifact\.mjs --profile=test/);
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
  };
  try {
    delete process.env.VITE_CONTROL_PLANE_BASE_URL;
    delete process.env.VITE_CONTROL_PLANE_ENV;
    delete process.env.AUTOLIVE_TAURI_CONFIG;
    applyBuildProfile('test');
    assert.equal(TEST_CONTROL_PLANE_BASE_URL, 'http://101.96.208.132:9090');
    assert.equal(process.env.VITE_CONTROL_PLANE_BASE_URL, TEST_CONTROL_PLANE_BASE_URL);
    assert.equal(process.env.VITE_CONTROL_PLANE_ENV, 'test');
    assert.equal(process.env.AUTOLIVE_TAURI_CONFIG, 'src-tauri/tauri.test.conf.json');
    assert.deepEqual(tauriBuildArguments('x86_64-pc-windows-msvc').slice(0, 3), [
      'build',
      '--config',
      'src-tauri/tauri.test.conf.json',
    ]);
  } finally {
    for (const [key, value] of Object.entries({
      VITE_CONTROL_PLANE_BASE_URL: previous.baseUrl,
      VITE_CONTROL_PLANE_ENV: previous.environment,
      AUTOLIVE_TAURI_CONFIG: previous.config,
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
