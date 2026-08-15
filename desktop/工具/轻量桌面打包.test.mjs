import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { tauriBuildArguments } from './构建桌面产物.mjs';
import { archiveDesktopArtifacts } from './归档桌面产物.mjs';

const configPath = fileURLToPath(new URL('../src-tauri/tauri.conf.json', import.meta.url));
const packageJsonPath = fileURLToPath(new URL('../ui/package.json', import.meta.url));
const workflowPath = fileURLToPath(
  new URL('../../.github/workflows/desktop-package.yml', import.meta.url),
);

function assertInOrder(source, fragments) {
  assert.equal(typeof source, 'string', '缺少构建脚本');
  let previous = -1;
  for (const fragment of fragments) {
    const current = source.indexOf(fragment);
    assert.ok(current > previous, `期望 ${fragment} 出现在正确顺序`);
    previous = current;
  }
}

test('Tauri 只打包内置运行资源清单', () => {
  const config = JSON.parse(readFileSync(configPath, 'utf8'));

  assert.equal(config.bundle.active, true);
  assert.deepEqual(config.bundle.resources, ['runtime-resources.json']);
});

test('Windows 与 macOS 都使用原生 Tauri bundle', () => {
  const expected = ['build', '--config', 'src-tauri/tauri.conf.json'];

  for (const target of [
    'x86_64-apple-darwin',
    'aarch64-apple-darwin',
    'x86_64-pc-windows-msvc',
  ]) {
    assert.deepEqual(tauriBuildArguments(target), expected);
  }
});

test('归档器只复制原生 bundle，不复制大运行资源', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-lightweight-package-'));
  const bundleSourceDir = join(root, 'bundle');
  const packageRoot = join(root, 'package');
  mkdirSync(join(bundleSourceDir, 'msi'), { recursive: true });
  writeFileSync(join(bundleSourceDir, 'msi', 'autolive.msi'), 'native-installer');

  const destination = archiveDesktopArtifacts({
    targetTriple: 'x86_64-pc-windows-msvc',
    bundleSourceDir,
    packageRoot,
  });

  assert.equal(readFileSync(join(destination, 'msi', 'autolive.msi'), 'utf8'), 'native-installer');
  for (const name of ['portable', 'binaries', 'voice-worker', 'voice-models']) {
    assert.equal(existsSync(join(destination, name)), false);
  }
});

test('本地完整构建与 CI prepared-resources 入口顺序明确', () => {
  const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));

  assertInOrder(packageJson.scripts['tauri:build'], [
    '准备FFmpeg资源.mjs',
    '准备语音Worker资源.mjs',
    '准备语音模型资源.mjs',
    '生成运行资源发布树.mjs',
    '构建桌面产物.mjs',
  ]);
  assertInOrder(packageJson.scripts['tauri:build:prepared-resources'], [
    '准备FFmpeg资源.mjs',
    '准备语音Worker资源.mjs',
    '准备语音模型资源.mjs --assert-prepared',
    '生成运行资源发布树.mjs',
    '构建桌面产物.mjs',
  ]);
});

test('CI 公共模型、目标运行资源和桌面包分层且避免 OpenMP 绕过', () => {
  const workflow = readFileSync(workflowPath, 'utf8');

  assert.equal((workflow.match(/^  common-models:$/gm) ?? []).length, 1);
  assert.match(workflow, /^  common-models:\n(?:.|\n)*?runs-on: ubuntu-latest/m);
  assert.match(workflow, /name: runtime-common-\$\{\{ steps\.desktop-version\.outputs\.version \}\}/);
  assert.match(workflow, /needs: common-models/);
  assert.match(workflow, /tauri:build:prepared-resources/);
  assert.match(workflow, /准备语音模型资源\.mjs --assert-prepared/);
  assert.match(
    workflow,
    /name: runtime-resources-\$\{\{ steps\.desktop-version\.outputs\.version \}\}-\$\{\{ matrix\.target_triple \}\}/,
  );
  assert.match(
    workflow,
    /name: desktop-bundle-\$\{\{ steps\.desktop-version\.outputs\.version \}\}-\$\{\{ matrix\.target_triple \}\}/,
  );
  assert.doesNotMatch(workflow, /KMP_DUPLICATE_LIB_OK/);
});

test('CI 仅手动 main 部署已构建产物，强制 host key 并不覆盖已发布文件', () => {
  const workflow = readFileSync(workflowPath, 'utf8');
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
  assert.match(deployJob, /rsync[^\n]*--ignore-existing/);
  assert.match(deployJob, /publish-runtime-resources/);
  assert.match(
    deployJob,
    /if ! \[\[ "\$DEPLOY_PORT" =~ \^\[0-9\]\{1,4\}\$ \]\] \|\| \(\( DEPLOY_PORT < 1 \|\| DEPLOY_PORT > 9999 \)\); then[\s\S]*?exit 1[\s\S]*?fi/,
  );
  assert.doesNotMatch(workflow, /StrictHostKeyChecking=no|password/i);
  assert.doesNotMatch(deployJob, /cargo build|pnpm .*build|npm .*build|node .*\u6784\u5efa/);
});
