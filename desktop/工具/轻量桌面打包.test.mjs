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
const implementationPlanPath = fileURLToPath(
  new URL('../../docs/superpowers/plans/2026-08-15-桌面运行资源按需下载实施计划.md', import.meta.url),
);
const gitignorePath = fileURLToPath(new URL('../../.gitignore', import.meta.url));

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

  assert.equal(tauriBuildArguments.length, 0);
  assert.deepEqual(tauriBuildArguments(), expected);
});

test('归档器只复制原生 bundle，不复制大运行资源', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-lightweight-package-'));
  const bundleSourceDir = join(root, 'bundle');
  const packageRoot = join(root, 'package');
  mkdirSync(join(bundleSourceDir, 'msi'), { recursive: true });
  mkdirSync(join(bundleSourceDir, 'macos', 'autolive.app'), { recursive: true });
  writeFileSync(join(bundleSourceDir, 'msi', 'autolive.msi'), 'native-installer');
  writeFileSync(join(bundleSourceDir, 'macos', 'autolive.app', 'Contents.txt'), 'not-an-installer');

  const destination = archiveDesktopArtifacts({
    targetTriple: 'x86_64-pc-windows-msvc',
    bundleSourceDir,
    packageRoot,
  });

  assert.equal(readFileSync(join(destination, 'msi', 'autolive.msi'), 'utf8'), 'native-installer');
  for (const name of ['portable', 'binaries', 'voice-worker', 'voice-models']) {
    assert.equal(existsSync(join(destination, name)), false);
  }
  assert.equal(existsSync(join(destination, 'macos')), false);
});

test('归档器缺少目标平台安装文件时失败', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-lightweight-package-'));
  const bundleSourceDir = join(root, 'bundle');
  mkdirSync(join(bundleSourceDir, 'macos', 'autolive.app'), { recursive: true });
  writeFileSync(join(bundleSourceDir, 'macos', 'autolive.app', 'Contents.txt'), 'not-an-installer');

  assert.throws(
    () => archiveDesktopArtifacts({
      targetTriple: 'aarch64-apple-darwin',
      bundleSourceDir,
      packageRoot: join(root, 'package'),
    }),
    /找不到.*安装文件/,
  );
});

test('运行资源生成与 CI artifact 均从 Tauri 配置版本派生', async () => {
  const config = JSON.parse(readFileSync(configPath, 'utf8'));
  const workflow = readFileSync(workflowPath, 'utf8');
  const { RESOURCE_RELEASE } = await import('./生成运行资源发布树.mjs');

  assert.equal(RESOURCE_RELEASE, `v${config.version}`);
  assert.match(
    workflow,
    /desktop\/resource-release\/autolive-resources\/\$\{\{ steps\.desktop-version\.outputs\.version \}\}\/common\/\*\*/,
  );
  assert.match(
    workflow,
    /desktop\/resource-release\/autolive-resources\/\$\{\{ steps\.desktop-version\.outputs\.version \}\}\/\$\{\{ matrix\.target_triple \}\}\/\*\*/,
  );
  assert.doesNotMatch(workflow, /resource-release\/autolive-resources\/v0\.1\.0\//);
});

test('本地发布产物与内置清单不进入版本控制', () => {
  const gitignore = readFileSync(gitignorePath, 'utf8');

  assert.match(gitignore, /^desktop\/src-tauri\/runtime-resources\.json$/m);
  assert.match(gitignore, /^desktop\/resource-release\/$/m);
  assert.match(gitignore, /^desktop\/src-tauri\/binaries\/\*$/m);
  assert.match(gitignore, /^!desktop\/src-tauri\/binaries\/\.gitignore$/m);
  assert.match(gitignore, /^desktop\/src-tauri\/voice-models\/\*$/m);
  assert.match(gitignore, /^!desktop\/src-tauri\/voice-models\/\.gitkeep$/m);
  assert.match(gitignore, /^desktop\/src-tauri\/voice-worker\/\*$/m);
  assert.match(gitignore, /^!desktop\/src-tauri\/voice-worker\/\.gitkeep$/m);
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
  assert.match(
    workflow,
    /path: desktop\/resource-release\/autolive-resources\/\$\{\{ steps\.desktop-version\.outputs\.version \}\}\/common\/\*\*/,
  );
  assert.match(workflow, /needs: common-models/);
  assert.match(workflow, /tauri:build:prepared-resources/);
  assert.match(workflow, /准备语音模型资源\.mjs --assert-prepared/);
  assert.match(
    workflow,
    /name: runtime-resources-\$\{\{ steps\.desktop-version\.outputs\.version \}\}-\$\{\{ matrix\.target_triple \}\}/,
  );
  assert.match(
    workflow,
    /path: desktop\/resource-release\/autolive-resources\/\$\{\{ steps\.desktop-version\.outputs\.version \}\}\/\$\{\{ matrix\.target_triple \}\}\/\*\*/,
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
  assert.match(deployJob, /rsync --archive --compress --ignore-existing --mkpath/);
  assert.match(deployJob, /UPLOAD_ID: \$\{\{ github\.run_id \}\}-\$\{\{ github\.run_attempt \}\}/);
  assert.match(
    deployJob,
    /if ! \[\[ "\$UPLOAD_ID" =~ \^\[1-9\]\[0-9\]\*-\[1-9\]\[0-9\]\*\$ \]\]; then[\s\S]*?exit 1[\s\S]*?fi/,
  );
  assert.match(
    deployJob,
    /"\$DEPLOY_USER@\$DEPLOY_HOST:\$UPLOAD_ID\/\$RELEASE_VERSION\/\$scope\/"/,
  );
  assert.doesNotMatch(deployJob, /:\/fs\/autolive-resources-staging/);
  assert.match(deployJob, /publish-runtime-resources/);
  assert.match(
    deployJob,
    /for scope in common x86_64-apple-darwin aarch64-apple-darwin x86_64-pc-windows-msvc; do[\s\S]*?"publish-runtime-resources \$RELEASE_VERSION \$scope \$UPLOAD_ID"[\s\S]*?done/,
  );
  assert.match(
    deployJob,
    /concurrency:\n\s+group: deploy-runtime-resources-\$\{\{ needs\.common-models\.outputs\.version \}\}\n\s+cancel-in-progress: false/,
  );
  assert.match(
    deployJob,
    /if ! \[\[ "\$DEPLOY_PORT" =~ \^\[0-9\]\{1,4\}\$ \]\] \|\| \(\( DEPLOY_PORT < 1 \|\| DEPLOY_PORT > 9999 \)\); then[\s\S]*?exit 1[\s\S]*?fi/,
  );
  assert.doesNotMatch(workflow, /StrictHostKeyChecking=no|password/i);
  assert.doesNotMatch(deployJob, /cargo build|pnpm .*build|npm .*build|node .*\u6784\u5efa/);
});

test('Task 6 用 forced-command dispatcher 同时约束 rrsync 写入和精确发布命令', () => {
  const task6 = readFileSync(implementationPlanPath, 'utf8');
  assert.match(
    task6,
    /authorized_keys[^\n]*command="\/usr\/local\/sbin\/autolive-resource-deploy-dispatcher",restrict/,
  );
  assert.match(
    task6,
    /exec \/usr\/local\/lib\/autolive-resources\/rrsync -wo -no-overwrite -munge \/fs\/autolive-resources-staging/,
  );
  assert.match(task6, /rsync-3\.4\.4\.tar\.gz/);
  assert.match(task6, /bd88cf82fa653da32314fb229136407c5c90f80d1758d8f4b091767877d8fa96/);
  assert.match(task6, /7bc4950a886bc2f4986b8a85fe492b8b3612a0f7edab031ab79c66fca0390970/);
  assert.match(
    task6,
    /\/usr\/local\/lib\/autolive-resources\/rrsync -help \/fs\/autolive-resources-staging/,
  );
  assert.match(task6, /-help[\s\S]*-wo[\s\S]*-no-overwrite[\s\S]*-munge/);
  assert.match(task6, /rsync --server[^\n]*<upload-id>\/<release>\/<scope>\//);
  assert.match(task6, /只允许精确的 `publish-runtime-resources <release> <scope> <upload-id>`/);
  assert.match(task6, /upload-id[^\n]*`\^\[1-9\]\[0-9\]\*-\[1-9\]\[0-9\]\*\$`/);
  assert.match(task6, /publisher[^\n]*flock[^\n]*发布锁/);
  assert.match(task6, /run-scoped source[^\n]*原子重命名[^\n]*private candidate/);
  assert.match(task6, /candidate[^\n]*任何 symlink[^\n]*失败/);
  assert.match(
    task6,
    /按 `autolive-deploy-inventory.json`[^\n]*相对路径、regular-file 类型、大小和 SHA-256[^\n]*拒绝缺失文件和额外文件/,
  );
  assert.match(task6, /校验成功后删除 candidate 中的 inventory/);
  assert.match(task6, /旧 run staging 绝不能被新 run 复用或合并/);
  assert.match(
    task6,
    /发布树生成隔离副本时递归删除所有名称以 `\.` 开头的文件和目录，sourceRoot 保持不变/,
  );
  assert.match(
    task6,
    /`autolive-deploy-inventory\.json` 是 `validate_complete_inventory` 唯一允许但不自列的部署元数据；除此之外任何额外文件都必须 fail-closed/,
  );
  assert.match(
    task6,
    /重复发布返回成功后，CI 的 scope 循环继续处理后续 scope/,
  );
  assert.match(task6, /禁止普通登录 shell/);
  assert.match(task6, /release 只接受精确 `v0\.1\.0`/);
  assert.match(task6, /scope 只接受 `common` 和三个目标三元组/);
  assert.match(task6, /release、scope 和 upload-id 均显式拒绝 `\/`、`\\` 与额外字符/);
});
