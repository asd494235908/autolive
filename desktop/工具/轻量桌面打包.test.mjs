import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { tauriBuildArguments } from './构建桌面产物.mjs';
import { archiveDesktopArtifacts } from './归档桌面产物.mjs';
import { readDesktopVersion } from './桌面版本.mjs';

const configPath = fileURLToPath(new URL('../src-tauri/tauri.conf.json', import.meta.url));
const packageJsonPath = fileURLToPath(new URL('../ui/package.json', import.meta.url));
const workflowPath = fileURLToPath(
  new URL('../../.github/workflows/desktop-package.yml', import.meta.url),
);
const gitignorePath = fileURLToPath(new URL('../../.gitignore', import.meta.url));

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

function assertInOrder(source, fragments) {
  let previous = -1;
  for (const fragment of fragments) {
    const current = source.indexOf(fragment);
    assert.ok(current > previous, `期望 ${fragment} 出现在正确顺序`);
    previous = current;
  }
}

function createWindowsArchiveFixture(root, files) {
  const executableSourcePath = join(root, 'target', 'release', 'autolive-desktop-core.exe');
  const bundleSourceDir = join(root, 'target', 'release', 'bundle');
  const manifestPath = join(root, 'runtime-resources.json');
  const embeddedResourceDir = join(root, 'embedded-runtime-resources');
  mkdirSync(join(bundleSourceDir, 'nsis'), { recursive: true });
  mkdirSync(join(root, 'target', 'release'), { recursive: true });
  writeFileSync(executableSourcePath, 'tauri-production-exe');
  writeFileSync(join(bundleSourceDir, 'nsis', 'autolive-setup.exe'), 'nsis-installer');
  for (const file of files) {
    const path = join(embeddedResourceDir, ...file.relative_path.split('/'));
    mkdirSync(join(path, '..'), { recursive: true });
    writeFileSync(path, file.content);
  }
  writeFileSync(manifestPath, JSON.stringify({
    schema_version: 1,
    release: readDesktopVersion().release,
    target: 'x86_64-pc-windows-msvc',
    files: files.map(({ relative_path, content }) => ({
      relative_path,
      size_bytes: Buffer.byteLength(content),
      sha256: sha256(content),
      component: 'media',
      executable: true,
    })),
  }));
  return { bundleSourceDir, embeddedResourceDir, executableSourcePath, manifestPath };
}

test('Tauri 打包运行资源清单和内嵌媒体资源树', () => {
  const config = JSON.parse(readFileSync(configPath, 'utf8'));

  assert.equal(config.bundle.active, true);
  assert.deepEqual(config.bundle.resources, [
    'runtime-resources.json',
    'embedded-runtime-resources',
  ]);
});

test('Windows 生成 NSIS 安装 EXE', () => {
  assert.deepEqual(tauriBuildArguments('x86_64-pc-windows-msvc'), [
    'build',
    '--config',
    'src-tauri/tauri.conf.json',
    '--bundles',
    'nsis',
  ]);
});

test('macOS 仍可通过环境变量选择单一原生 bundle 类型', () => {
  const previous = process.env.AUTOLIVE_BUNDLES;
  process.env.AUTOLIVE_BUNDLES = 'dmg';
  try {
    assert.deepEqual(tauriBuildArguments('aarch64-apple-darwin'), [
      'build',
      '--config',
      'src-tauri/tauri.conf.json',
      '--bundles',
      'dmg',
    ]);
  } finally {
    if (previous === undefined) delete process.env.AUTOLIVE_BUNDLES;
    else process.env.AUTOLIVE_BUNDLES = previous;
  }
});

test('Windows 归档器同时交付 NSIS EXE 与 media-only portable', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-media-package-'));
  const relativePath = 'x86_64-pc-windows-msvc/binaries/ffmpeg.exe';
  const fixture = createWindowsArchiveFixture(root, [{ relative_path: relativePath, content: 'ffmpeg' }]);
  const destination = archiveDesktopArtifacts({
    targetTriple: 'x86_64-pc-windows-msvc',
    ...fixture,
    packageRoot: join(root, 'package'),
    createPortableZip: process.platform === 'win32',
  });

  assert.equal(
    readFileSync(join(destination, 'portable', 'autolive-desktop-core.exe'), 'utf8'),
    'tauri-production-exe',
  );
  assert.equal(
    readFileSync(join(destination, 'portable', 'embedded-runtime-resources', ...relativePath.split('/')), 'utf8'),
    'ffmpeg',
  );
  assert.equal(readFileSync(join(destination, 'nsis', 'autolive-setup.exe'), 'utf8'), 'nsis-installer');
  assert.equal(existsSync(join(destination, 'msi')), false);
  assert.equal(existsSync(join(destination, 'portable', 'embedded-runtime-resources', 'common')), false);
  if (process.platform === 'win32') {
    const archive = join(
      destination,
      `autolive-desktop-core_${readDesktopVersion().version}_x64-portable-with-resources.zip`,
    );
    assert.equal(readFileSync(archive).subarray(0, 2).toString('ascii'), 'PK');
  }
});

test('Windows 归档器缺少正式 Tauri EXE 时失败', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-media-package-'));
  const fixture = createWindowsArchiveFixture(root, [{
    relative_path: 'x86_64-pc-windows-msvc/binaries/ffmpeg.exe',
    content: 'ffmpeg',
  }]);

  assert.throws(
    () => archiveDesktopArtifacts({
      targetTriple: 'x86_64-pc-windows-msvc',
      ...fixture,
      executableSourcePath: join(root, 'missing.exe'),
      packageRoot: join(root, 'package'),
      createPortableZip: false,
    }),
    /找不到正式 Tauri EXE/,
  );
});

test('Windows 归档器拒绝包含重复路径的运行资源清单', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-media-package-'));
  const relativePath = 'x86_64-pc-windows-msvc/binaries/ffmpeg.exe';
  const fixture = createWindowsArchiveFixture(root, [
    { relative_path: relativePath, content: 'ffmpeg' },
  ]);
  const manifest = JSON.parse(readFileSync(fixture.manifestPath, 'utf8'));
  manifest.files.push(manifest.files[0]);
  writeFileSync(fixture.manifestPath, JSON.stringify(manifest));

  assert.throws(
    () => archiveDesktopArtifacts({
      targetTriple: 'x86_64-pc-windows-msvc',
      ...fixture,
      packageRoot: join(root, 'package'),
      createPortableZip: false,
    }),
    /重复路径/,
  );
});

test('本地构建只准备 FFmpeg、生成发布树并构建桌面产物', () => {
  const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));

  for (const name of ['tauri:build', 'tauri:build:prepared-resources']) {
    assertInOrder(packageJson.scripts[name], [
      '准备FFmpeg资源.mjs',
      '生成运行资源发布树.mjs',
      '构建桌面产物.mjs',
    ]);
    assert.doesNotMatch(packageJson.scripts[name], /语音|voice|model/i);
  }
});

test('CI 不再构建或发布固定话术 Worker 和模型', () => {
  const workflow = readFileSync(workflowPath, 'utf8').replaceAll('\r\n', '\n');

  assert.match(workflow, /^  metadata:$/m);
  assert.match(workflow, /tauri:build:prepared-resources/);
  assert.match(workflow, /name: desktop-bundle-/);
  assert.doesNotMatch(workflow, /common-models|runtime-common|voice.clone|Coqui|requirements-voice|Setup Python|pip install/i);
  assert.match(
    workflow,
    /for scope in x86_64-apple-darwin aarch64-apple-darwin x86_64-pc-windows-msvc; do/,
  );
  assert.doesNotMatch(workflow, /for scope in common/);
});

test('CI 只手动从 main 部署已构建资源且强制 host key', () => {
  const workflow = readFileSync(workflowPath, 'utf8').replaceAll('\r\n', '\n');
  const deployJob = workflow.slice(workflow.indexOf('\n  deploy:'));

  assert.match(workflow, /^permissions:\n  contents: read$/m);
  assert.match(
    deployJob,
    /if: github\.event_name == 'workflow_dispatch' && github\.ref == 'refs\/heads\/main'/,
  );
  assert.doesNotMatch(deployJob, /actions\/checkout/);
  assert.match(deployJob, /AUTOLIVE_RESOURCE_DEPLOY_KEY/);
  assert.match(deployJob, /AUTOLIVE_RESOURCE_DEPLOY_HOST_KEY/);
  assert.match(deployJob, /StrictHostKeyChecking=yes/);
  assert.match(deployJob, /rsync --archive --compress --ignore-existing --mkpath/);
  assert.match(deployJob, /publish-runtime-resources/);
  assert.doesNotMatch(workflow, /StrictHostKeyChecking=no|password/i);
  assert.doesNotMatch(deployJob, /cargo build|pnpm .*build|npm .*build/);
});

test('本地发布与 smoke 产物不进入版本控制', () => {
  const gitignore = readFileSync(gitignorePath, 'utf8');

  assert.match(gitignore, /^desktop\/src-tauri\/runtime-resources\.json$/m);
  assert.match(gitignore, /^desktop\/src-tauri\/embedded-runtime-resources\/$/m);
  assert.match(gitignore, /^desktop\/resource-release\/$/m);
  assert.match(gitignore, /^desktop\/src-tauri\/binaries\/\*$/m);
  assert.match(gitignore, /^desktop\/package-smoke\/$/m);
  assert.match(gitignore, /^desktop\/\.codex-worker-smoke\/$/m);
});
