import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { verifyReproducibleBuildLock } from './verify-mpv-reproducible-build-lock.mjs';
import { completeBuildLockFixture } from './mpv-reproducible-build-test-fixtures.mjs';

const lockUrl = new URL('../third_party/mpv/reproducible-build-lock.json', import.meta.url);
const verifierUrl = new URL('./verify-mpv-reproducible-build-lock.mjs', import.meta.url);
const verifierPath = fileURLToPath(verifierUrl);

async function repositoryLock() {
  return JSON.parse(await readFile(lockUrl, 'utf8'));
}

async function completeFixture() {
  return completeBuildLockFixture(await repositoryLock());
}

test('仓库锁由校验器派生 metadata_complete，不含自证完成字段', async () => {
  const lock = await repositoryLock();
  for (const field of ['status', 'audit', 'unresolved', 'required_outputs']) {
    assert.equal(Object.hasOwn(lock, field), false, `${field} 不应进入输入锁`);
  }
  const report = verifyReproducibleBuildLock(lock);
  assert.equal(report.checkStatus, 'passed');
  assert.equal(report.status, 'metadata_complete');
  assert.equal(report.metadataComplete, true);
  assert.equal(report.target, 'x86_64-pc-windows-msvc');
  assert.equal(report.sourceCount, lock.sources.length);
  assert.deepEqual(report.blockers, []);
});

test('固定全部声明输入和断网配方后，输入锁元数据才 complete', async () => {
  const report = verifyReproducibleBuildLock(await completeFixture());
  assert.equal(report.status, 'metadata_complete');
  assert.equal(report.metadataComplete, true);
  assert.deepEqual(report.blockers, []);
});

test('Git 与归档输入使用不同身份契约', async () => {
  const git = await completeFixture();
  git.sources[0].commit = 'main';
  assert.throws(() => verifyReproducibleBuildLock(git), /40 位提交/);

  const archive = await completeFixture();
  archive.sources.at(-1).commit = 'a'.repeat(40);
  assert.throws(() => verifyReproducibleBuildLock(archive), /字段必须精确匹配/);
});

test('输入漂移、联网、自动能力与音频输出遗漏均失败关闭', async () => {
  const cases = [
    ['重复源码', (lock) => { lock.sources.push(structuredClone(lock.sources[0])); }, /重复组件/],
    ['缓存缺哈希', (lock) => { lock.sources[0].sha256 = null; }, /同时存在/],
    ['缓存越界', (lock) => { lock.sources[0].cache_path = 'sources/../mpv.tar.zst'; }, /无效路径段/],
    ['Windows 绝对路径', (lock) => { lock.sources[0].cache_path = 'C:/sources/mpv.tar.zst'; }, /相对路径/],
    ['缓存路径复用', (lock) => { lock.sources[1].cache_path = lock.sources[0].cache_path; lock.cache_inventory = [...new Set(lock.cache_inventory.filter((item) => item !== 'sources/ffmpeg.tar.zst'))]; }, /复用缓存路径/],
    ['工具镜像无摘要', (lock) => { lock.toolchain.builder_image = 'ghcr.io/example/mpv-builder:latest'; }, /镜像摘要/],
    ['工具来源非 HTTPS', (lock) => { lock.toolchain.tools.clang.source = 'http://example.invalid/clang'; }, /HTTPS/],
    ['工具来源带凭证', (lock) => { lock.toolchain.tools.clang.source = 'https://user:secret@example.invalid/clang'; }, /无凭证/],
    ['镜像工具伪造独立缓存', (lock) => { lock.toolchain.tools.clang.cache_path = 'toolchain/clang.tar.zst'; lock.toolchain.tools.clang.size_bytes = 1; lock.toolchain.tools.clang.sha256 = 'a'.repeat(64); }, /不得声明独立缓存/],
    ['缺少 MSVC CRT STL 工具集', (lock) => { delete lock.toolchain.tools['msvc-toolset']; }, /固定契约/],
    ['移动归档 URL', (lock) => { lock.sources.at(-1).url = 'https://example.invalid/latest/shaderc.tar.zst'; }, /移动引用/],
    ['构建参数含 URL', (lock) => { lock.build_recipe.meson_arguments.push('https://example.invalid'); }, /禁止联网/],
    ['构建参数重复', (lock) => { lock.build_recipe.meson_arguments.push('-Dwasapi=disabled'); }, /不允许重复/],
    ['相反 WASAPI 参数', (lock) => { lock.build_recipe.meson_arguments.push('-Dwasapi=enabled'); }, /未知或覆盖/],
    ['相反自动能力参数', (lock) => { lock.build_recipe.meson_arguments.push('-Dauto_features=auto'); }, /未知或覆盖/],
    ['相反 FFmpeg 参数', (lock) => { lock.build_recipe.ffmpeg_arguments.push('--enable-programs'); }, /未知或覆盖/],
    ['构建参数含同步脚本', (lock) => { lock.build_recipe.meson_arguments.push('git-sync-deps'); }, /禁止联网/],
    ['FFmpeg 开网络', (lock) => { lock.build_recipe.ffmpeg_arguments.push('--enable-network'); }, /未知或覆盖/],
    ['FFmpeg GPL 身份漂移', (lock) => { lock.sources.find(({ name }) => name === 'ffmpeg').license_expression = 'LGPL-2.1-or-later'; }, /FFmpeg.*GPL/],
    ['自动能力恢复', (lock) => { lock.features.auto_features = 'auto'; }, /auto_features/],
    ['构建日期恢复', (lock) => { lock.features.build_date = true; }, /build_date/],
    ['缺 WASAPI 禁用', (lock) => { lock.build_recipe.meson_arguments = lock.build_recipe.meson_arguments.filter((item) => item !== '-Dwasapi=disabled'); }, /wasapi/],
    ['额外根字段', (lock) => { lock.status = 'complete'; }, /字段必须精确匹配/],
    ['配方入口同一文件', (lock) => { lock.build_recipe.script_path = lock.toolchain.dockerfile_path; lock.build_recipe.script_sha256 = lock.toolchain.dockerfile_sha256; }, /不能是同一文件/],
  ];
  for (const [name, mutate, pattern] of cases) {
    const lock = await completeFixture();
    mutate(lock);
    const verify = () => verifyReproducibleBuildLock(lock);
    if (name === '缺 WASAPI 禁用') {
      assert.match(verify().blockers.join('\n'), pattern, name);
    } else {
      assert.throws(verify, pattern, name);
    }
  }
});

test('缓存清单必须与所有源码、补丁和工具链输入双向相等', async () => {
  const missing = await completeFixture();
  missing.cache_inventory.pop();
  assert.match(verifyReproducibleBuildLock(missing).blockers.join('\n'), /cache_inventory/);

  const extra = await completeFixture();
  extra.cache_inventory.push('sources/undeclared.tar.zst');
  assert.match(verifyReproducibleBuildLock(extra).blockers.join('\n'), /cache_inventory/);
});

test('CLI 默认报告 complete，完整模式要求输入锁元数据 complete', () => {
  const normal = spawnSync(process.execPath, [verifierPath], { encoding: 'utf8' });
  assert.equal(normal.status, 0);
  assert.equal(JSON.parse(normal.stdout).status, 'metadata_complete');

  const complete = spawnSync(process.execPath, [verifierPath, '--require-metadata-complete'], { encoding: 'utf8' });
  assert.equal(complete.status, 0);
  assert.equal(JSON.parse(complete.stdout).metadataComplete, true);

  const unknown = spawnSync(process.execPath, [verifierPath, '--unknown'], { encoding: 'utf8' });
  assert.equal(unknown.status, 1);
  assert.match(unknown.stderr, /--require-metadata-complete/);
});

test('静态门禁不执行进程、不联网，也进入桌面默认测试入口', async () => {
  const [source, packageSource] = await Promise.all([
    readFile(verifierUrl, 'utf8'),
    readFile(new URL('../ui/package.json', import.meta.url), 'utf8'),
  ]);
  assert.doesNotMatch(source, /node:child_process|\bfetch\s*\(|node:https?|\b(?:exec|spawn)\s*\(/);
  const packageJson = JSON.parse(packageSource);
  assert.equal(packageJson.scripts['test:mpv-reproducible-lock'],
    'node --test ../tools/verify-mpv-reproducible-build-lock.test.mjs');
  assert.equal(packageJson.scripts['audit:mpv-reproducible-lock'],
    'node ../tools/verify-mpv-reproducible-build-lock.mjs');
  assert.match(packageJson.scripts.test, /verify-mpv-reproducible-build-lock\.test\.mjs/);
});
