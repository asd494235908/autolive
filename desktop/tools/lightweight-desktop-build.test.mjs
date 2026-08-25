import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import {
  cleanBundleOutputForTarget,
  tauriBuildArguments,
} from './build-desktop-artifact.mjs';
import { archiveDesktopArtifacts } from './archive-desktop-artifact.mjs';
import { readDesktopVersion } from './desktop-version.mjs';

const configPath = fileURLToPath(new URL('../src-tauri/tauri.conf.json', import.meta.url));
const nsisHooksPath = fileURLToPath(new URL('../src-tauri/windows/hooks.nsh', import.meta.url));
const packageJsonPath = fileURLToPath(new URL('../ui/package.json', import.meta.url));
const buildScriptPath = fileURLToPath(new URL('./build-desktop-artifact.mjs', import.meta.url));
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
  assert.ok(config.bundle.resources.includes('runtime-resources.json'));
  assert.ok(config.bundle.resources.includes('embedded-runtime-resources'));
  assert.ok(config.bundle.resources.includes('portaudio/portaudio_x64.dll'));
  assert.ok(config.bundle.resources.includes('portaudio/LICENSE.txt'));
  assert.ok(config.bundle.resources.includes('signalsmith-stretch/LICENSE.txt'));
  assert.ok(config.bundle.resources.includes('signalsmith-linear/LICENSE.txt'));
  assert.equal(config.bundle.windows.nsis.installerHooks, './windows/hooks.nsh');
  assert.deepEqual(config.bundle.windows.webviewInstallMode, {
    type: 'offlineInstaller',
    silent: true,
  });
});

test('NSIS 安装后将 PortAudio DLL 放到 EXE 同目录并在卸载前清理', () => {
  const hooks = readFileSync(nsisHooksPath, 'utf8');

  assert.match(hooks, /NSIS_HOOK_POSTINSTALL/);
  assert.match(hooks, /\$INSTDIR\\portaudio\\portaudio_x64\.dll/);
  assert.match(hooks, /CopyFiles \/SILENT/);
  assert.match(hooks, /\$INSTDIR\\portaudio_x64\.dll/);
  assert.match(hooks, /NSIS_HOOK_PREUNINSTALL/);
  assert.match(hooks, /Delete "\$INSTDIR\\portaudio_x64\.dll"/);
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

test('Windows 构建前只清理当前 NSIS bundle 输出目录', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-bundle-cleanup-'));
  const releaseRoot = join(root, 'target', 'release');
  const bundleRoot = join(releaseRoot, 'bundle');
  const nsisDir = join(bundleRoot, 'nsis');
  const msiDir = join(bundleRoot, 'msi');
  mkdirSync(nsisDir, { recursive: true });
  mkdirSync(msiDir, { recursive: true });
  writeFileSync(join(nsisDir, 'stale-setup.exe'), 'stale');
  writeFileSync(join(msiDir, 'keep.msi'), 'keep');
  writeFileSync(join(releaseRoot, 'keep.exe'), 'keep');

  assert.equal(
    cleanBundleOutputForTarget('x86_64-pc-windows-msvc', bundleRoot),
    nsisDir,
  );
  assert.equal(existsSync(nsisDir), false);
  assert.equal(existsSync(join(msiDir, 'keep.msi')), true);
  assert.equal(existsSync(join(releaseRoot, 'keep.exe')), true);

  const buildScript = readFileSync(buildScriptPath, 'utf8');
  const buildFunction = buildScript.slice(buildScript.indexOf('export function buildDesktopArtifacts'));
  assertInOrder(buildFunction, [
    'cleanBundleOutputForTarget(targetTriple)',
    "spawnSync('tauri'",
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
  const ambientResourceDir = join(root, 'ambient');
  const relativePath = 'x86_64-pc-windows-msvc/binaries/ffmpeg.exe';
  const fixture = createWindowsArchiveFixture(root, [{ relative_path: relativePath, content: 'ffmpeg' }]);
  mkdirSync(ambientResourceDir, { recursive: true });
  writeFileSync(join(ambientResourceDir, 'low-level-room-tone.wav'), 'room-tone');
  writeFileSync(join(ambientResourceDir, 'LICENSE.txt'), 'ambient-license');
  writeFileSync(join(ambientResourceDir, 'unexpected.tmp'), 'must-not-be-copied');
  const destination = archiveDesktopArtifacts({
    targetTriple: 'x86_64-pc-windows-msvc',
    ...fixture,
    ambientResourceDir,
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
  assert.equal(
    readFileSync(join(destination, 'portable', 'ambient', 'low-level-room-tone.wav'), 'utf8'),
    'room-tone',
  );
  assert.equal(
    readFileSync(join(destination, 'portable', 'ambient', 'LICENSE.txt'), 'utf8'),
    'ambient-license',
  );
  assert.equal(existsSync(join(destination, 'portable', 'ambient', 'unexpected.tmp')), false);
  assert.equal(existsSync(join(destination, 'msi')), false);
  assert.equal(existsSync(join(destination, 'portable', 'embedded-runtime-resources', 'common')), false);
  if (process.platform === 'win32') {
    const archive = join(
      destination,
      `autolive-desktop-core_${readDesktopVersion().version}_x64-portable-with-resources.zip`,
    );
    assert.equal(readFileSync(archive).subarray(0, 2).toString('ascii'), 'PK');
    const listing = spawnSync('tar.exe', ['-tf', archive], { encoding: 'utf8' });
    assert.equal(listing.status, 0, listing.stderr);
    assert.match(listing.stdout, /portable\/ambient\/low-level-room-tone\.wav/);
    assert.match(listing.stdout, /portable\/ambient\/LICENSE\.txt/);
    assert.doesNotMatch(listing.stdout, /unexpected\.tmp/);
  }
});

test('Windows portable 归档缺少固定环境声资源时失败', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-media-package-'));
  const ambientResourceDir = join(root, 'ambient');
  const fixture = createWindowsArchiveFixture(root, [{
    relative_path: 'x86_64-pc-windows-msvc/binaries/ffmpeg.exe',
    content: 'ffmpeg',
  }]);
  mkdirSync(ambientResourceDir, { recursive: true });
  writeFileSync(join(ambientResourceDir, 'low-level-room-tone.wav'), 'room-tone');

  assert.throws(
    () => archiveDesktopArtifacts({
      targetTriple: 'x86_64-pc-windows-msvc',
      ...fixture,
      ambientResourceDir,
      packageRoot: join(root, 'package'),
      createPortableZip: false,
    }),
    /内置环境声资源缺失.*LICENSE\.txt/,
  );
});

test('Windows 归档器拒绝 NSIS 目录中的多个 EXE', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-media-package-'));
  const fixture = createWindowsArchiveFixture(root, [{
    relative_path: 'x86_64-pc-windows-msvc/binaries/ffmpeg.exe',
    content: 'ffmpeg',
  }]);
  writeFileSync(join(fixture.bundleSourceDir, 'nsis', 'stale-setup.exe'), 'stale-installer');

  assert.throws(
    () => archiveDesktopArtifacts({
      targetTriple: 'x86_64-pc-windows-msvc',
      ...fixture,
      packageRoot: join(root, 'package'),
      createPortableZip: false,
    }),
    /Windows NSIS.*恰好一个 EXE.*2 个/,
  );
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
      'prepare-ffmpeg-resources.mjs',
      'generate-runtime-resource-tree.mjs',
      'build-desktop-artifact.mjs',
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
