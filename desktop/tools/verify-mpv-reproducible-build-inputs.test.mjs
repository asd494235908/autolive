import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { completeBuildLockFixture } from './mpv-reproducible-build-test-fixtures.mjs';
import { runReproducibleBuildInputsGate } from './verify-mpv-reproducible-build-inputs.mjs';

const repositoryLockUrl = new URL('../third_party/mpv/reproducible-build-lock.json', import.meta.url);
const verifierUrl = new URL('./verify-mpv-reproducible-build-inputs.mjs', import.meta.url);
const verifierPath = fileURLToPath(verifierUrl);

function digest(value) {
  return createHash('sha256').update(value).digest('hex');
}

async function materializeFixture() {
  const root = await mkdtemp(join(tmpdir(), 'autolive-mpv-inputs-'));
  const repositoryLock = JSON.parse(await readFile(repositoryLockUrl, 'utf8'));
  const lock = completeBuildLockFixture(repositoryLock);
  const lockPath = join(root, 'desktop', 'third_party', 'mpv', 'reproducible-build-lock.json');
  const buildInputsRoot = join(root, 'desktop', 'third_party', 'mpv', 'build-inputs');
  const cacheOwners = [
    ...lock.sources,
    ...Object.values(lock.toolchain.tools),
  ];
  for (const owner of cacheOwners.filter((candidate) => candidate.cache_path !== null)) {
    const content = Buffer.from(`${owner.cache_path}\n`);
    owner.size_bytes = content.length;
    owner.sha256 = digest(content);
    const path = join(buildInputsRoot, ...owner.cache_path.split('/'));
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, content);
  }
  for (const recipe of lock.recipe_inventory) {
    const content = Buffer.from(`${recipe.path}\n`);
    const path = join(root, ...recipe.path.split('/'));
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, content);
    recipe.size_bytes = content.length;
    recipe.sha256 = digest(content);
    const patch = lock.patches.find((item) => item.path === recipe.path);
    if (patch) {
      patch.size_bytes = recipe.size_bytes;
      patch.sha256 = recipe.sha256;
    }
  }
  lock.toolchain.dockerfile_sha256 = lock.recipe_inventory.find(
    (recipe) => recipe.path === lock.toolchain.dockerfile_path,
  ).sha256;
  lock.build_recipe.script_sha256 = lock.recipe_inventory.find(
    (recipe) => recipe.path === lock.build_recipe.script_path,
  ).sha256;

  await mkdir(dirname(lockPath), { recursive: true });
  await writeFile(lockPath, `${JSON.stringify(lock, null, 2)}\n`);
  return { root, lock, lockPath, buildInputsRoot };
}

test('仓库当前固定输入逐字节匹配输入锁', async () => {
  const report = await runReproducibleBuildInputsGate();
  assert.equal(report.checkStatus, 'passed');
  assert.equal(report.status, 'input_bytes_match_lock');
  assert.equal(report.metadataComplete, true);
  assert.equal(report.inputBytesMatchLock, true);
  assert.equal(report.admitted, false);
});

test('完整元数据必须对应真实且哈希一致的缓存、Dockerfile 和构建脚本', async () => {
  const fixture = await materializeFixture();
  try {
    const report = await runReproducibleBuildInputsGate({
      lockFilePath: fixture.lockPath,
      workspaceRootPath: fixture.root,
      buildInputsRootPath: fixture.buildInputsRoot,
    });
    assert.equal(report.status, 'input_bytes_match_lock');
    assert.equal(report.metadataComplete, true);
    assert.equal(report.inputBytesMatchLock, true);
    assert.equal(report.admitted, false);
    assert.equal(report.cacheFileCount, fixture.lock.cache_inventory.length);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('缓存篡改、夹带和构建脚本漂移都失败关闭', async () => {
  const tampered = await materializeFixture();
  try {
    const path = join(tampered.buildInputsRoot, ...tampered.lock.cache_inventory[0].split('/'));
    await writeFile(path, 'tampered');
    await assert.rejects(
      runReproducibleBuildInputsGate({
        lockFilePath: tampered.lockPath,
        workspaceRootPath: tampered.root,
        buildInputsRootPath: tampered.buildInputsRoot,
      }),
      /缓存(?:大小| SHA-256)不匹配/,
    );
  } finally {
    await rm(tampered.root, { recursive: true, force: true });
  }

  const extra = await materializeFixture();
  try {
    await writeFile(join(extra.buildInputsRoot, 'extra.bin'), 'extra');
    await assert.rejects(
      runReproducibleBuildInputsGate({
        lockFilePath: extra.lockPath,
        workspaceRootPath: extra.root,
        buildInputsRootPath: extra.buildInputsRoot,
      }),
      /文件集合/,
    );
  } finally {
    await rm(extra.root, { recursive: true, force: true });
  }

  const script = await materializeFixture();
  try {
    const scriptPath = join(script.root, ...script.lock.build_recipe.script_path.split('/'));
    await writeFile(scriptPath, 'changed');
    await assert.rejects(
      runReproducibleBuildInputsGate({
        lockFilePath: script.lockPath,
        workspaceRootPath: script.root,
        buildInputsRootPath: script.buildInputsRoot,
      }),
      /构建配方(?:大小| SHA-256)不匹配/,
    );
  } finally {
    await rm(script.root, { recursive: true, force: true });
  }

  const recipe = await materializeFixture();
  try {
    await writeFile(join(recipe.root, 'desktop', 'third_party', 'mpv', 'build', 'undeclared.ps1'), 'extra');
    await assert.rejects(
      runReproducibleBuildInputsGate({
        lockFilePath: recipe.lockPath,
        workspaceRootPath: recipe.root,
        buildInputsRootPath: recipe.buildInputsRoot,
      }),
      /构建配方文件集合/,
    );
  } finally {
    await rm(recipe.root, { recursive: true, force: true });
  }
});

test('声明缓存总量超过 32 GiB 时在读取目录前阻断', async () => {
  const root = await mkdtemp(join(tmpdir(), 'autolive-mpv-input-limit-'));
  try {
    const repositoryLock = JSON.parse(await readFile(repositoryLockUrl, 'utf8'));
    const lock = completeBuildLockFixture(repositoryLock);
    for (const source of lock.sources) source.size_bytes = 8 * 1024 * 1024 * 1024;
    for (const tool of Object.values(lock.toolchain.tools)) {
      if (tool.provision === 'cache') tool.size_bytes = 8 * 1024 * 1024 * 1024;
    }
    const lockPath = join(root, 'lock.json');
    await writeFile(lockPath, JSON.stringify(lock));
    await assert.rejects(
      runReproducibleBuildInputsGate({
        lockFilePath: lockPath,
        workspaceRootPath: root,
        buildInputsRootPath: join(root, 'missing-inputs'),
      }),
      /总字节数超过上限/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('缓存符号链接在平台允许创建时必须阻断', async (context) => {
  const fixture = await materializeFixture();
  try {
    const path = join(fixture.buildInputsRoot, ...fixture.lock.cache_inventory[0].split('/'));
    const outside = join(fixture.root, 'outside.bin');
    await writeFile(outside, 'outside');
    await rm(path);
    try {
      await symlink(outside, path, 'file');
    } catch (error) {
      if (error?.code === 'EPERM' || error?.code === 'EACCES') {
        context.skip(`当前 Windows 环境不允许创建测试符号链接：${error.code}`);
        return;
      }
      throw error;
    }
    await assert.rejects(
      runReproducibleBuildInputsGate({
        lockFilePath: fixture.lockPath,
        workspaceRootPath: fixture.root,
        buildInputsRootPath: fixture.buildInputsRoot,
      }),
      /不允许符号链接/,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('CLI 默认报告 verified，文件模式要求 verified', () => {
  const normal = spawnSync(process.execPath, [verifierPath], { encoding: 'utf8' });
  assert.equal(normal.status, 0);
  assert.equal(JSON.parse(normal.stdout).status, 'input_bytes_match_lock');

  const verified = spawnSync(process.execPath, [verifierPath, '--require-verified'], { encoding: 'utf8' });
  assert.equal(verified.status, 0);
  assert.equal(JSON.parse(verified.stdout).inputBytesMatchLock, true);

  const unknown = spawnSync(process.execPath, [verifierPath, '--unknown'], { encoding: 'utf8' });
  assert.equal(unknown.status, 1);
  assert.match(unknown.stderr, /--require-verified/);
});

test('输入文件门禁不执行进程、不联网，也进入桌面默认测试入口', async () => {
  const [source, packageSource] = await Promise.all([
    readFile(verifierUrl, 'utf8'),
    readFile(new URL('../ui/package.json', import.meta.url), 'utf8'),
  ]);
  assert.doesNotMatch(source, /node:child_process|\bfetch\s*\(|node:https?|\b(?:exec|spawn)\s*\(/);
  const packageJson = JSON.parse(packageSource);
  assert.equal(packageJson.scripts['test:mpv-reproducible-inputs'],
    'node --test ../tools/verify-mpv-reproducible-build-inputs.test.mjs');
  assert.equal(packageJson.scripts['audit:mpv-reproducible-inputs'],
    'node ../tools/verify-mpv-reproducible-build-inputs.mjs');
  assert.match(packageJson.scripts.test, /verify-mpv-reproducible-build-inputs\.test\.mjs/);
});
