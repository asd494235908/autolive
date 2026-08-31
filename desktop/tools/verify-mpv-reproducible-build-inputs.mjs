import { isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { verifyReproducibleBuildLock } from './verify-mpv-reproducible-build-lock.mjs';
import {
  inspectAndHashFile,
  listRegularFiles,
  readBoundedRegularFile,
} from './verify-mpv-reproducible-build-report.mjs';

const workspaceRoot = fileURLToPath(new URL('../../', import.meta.url));
const thirdPartyRoot = fileURLToPath(new URL('../third_party/mpv/', import.meta.url));
const defaultLockPath = join(thirdPartyRoot, 'reproducible-build-lock.json');
const defaultBuildInputsRoot = join(thirdPartyRoot, 'build-inputs');
const MAX_LOCK_BYTES = 4 * 1024 * 1024;
const MAX_BUILD_INPUT_TOTAL_BYTES = 32 * 1024 * 1024 * 1024;

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function exactStringSet(actualValue, expectedValue, path) {
  const actual = [...actualValue].sort();
  const expected = [...expectedValue].sort();
  assert(
    actual.length === expected.length && actual.every((item, index) => item === expected[index]),
    `${path} 与输入锁不完全一致`,
  );
}

function pathWithin(root, candidate) {
  const child = relative(root, candidate);
  return child !== '' && !child.startsWith('..') && !isAbsolute(child);
}

function cacheEvidence(lock) {
  return new Map([
    ...lock.sources.map((source) => [source.cache_path, { size: source.size_bytes, sha256: source.sha256 }]),
    ...Object.values(lock.toolchain.tools)
      .filter((tool) => tool.provision === 'cache')
      .map((tool) => [tool.cache_path, { size: tool.size_bytes, sha256: tool.sha256 }]),
  ]);
}

export async function runReproducibleBuildInputsGate({
  lockFilePath = defaultLockPath,
  workspaceRootPath = workspaceRoot,
  buildInputsRootPath = defaultBuildInputsRoot,
} = {}) {
  const lockBytes = await readBoundedRegularFile(lockFilePath, MAX_LOCK_BYTES, '输入锁');
  const lock = JSON.parse(lockBytes.toString('utf8'));
  const metadata = verifyReproducibleBuildLock(lock);
  if (!metadata.metadataComplete) {
    return {
      schemaVersion: 1,
      gate: 'mpv-reproducible-build-inputs',
      checkStatus: 'passed',
      status: 'blocked',
      metadataComplete: false,
      inputBytesMatchLock: false,
      admitted: false,
      blockers: [...metadata.blockers, '构建输入元数据尚未完成，未读取缓存文件'],
    };
  }

  const evidence = cacheEvidence(lock);
  assert(evidence.size === lock.cache_inventory.length, '缓存证据映射与 inventory 数量不一致');
  const declaredBytes = [...evidence.values()].reduce((total, item) => total + item.size, 0);
  assert(declaredBytes <= MAX_BUILD_INPUT_TOTAL_BYTES, '构建输入缓存总字节数超过上限');
  const inventory = await listRegularFiles(resolve(buildInputsRootPath), {
    maximumFiles: 512,
    maximumEntries: 768,
    maximumDirectories: 128,
    maximumDepth: 8,
  });
  exactStringSet(inventory.files, lock.cache_inventory, 'build-inputs 文件集合');
  for (const path of lock.cache_inventory) {
    const absolute = resolve(buildInputsRootPath, ...path.split('/'));
    assert(pathWithin(resolve(buildInputsRootPath), absolute), `缓存路径越界：${path}`);
    const observed = await inspectAndHashFile(absolute, inventory.canonicalRoot, evidence.get(path).size);
    assert(observed.size === evidence.get(path).size, `缓存大小不匹配：${path}`);
    assert(observed.sha256 === evidence.get(path).sha256, `缓存 SHA-256 不匹配：${path}`);
  }
  const recipeRoot = resolve(workspaceRootPath, 'desktop', 'third_party', 'mpv', 'build');
  const recipeInventory = await listRegularFiles(recipeRoot, {
    maximumFiles: 64,
    maximumEntries: 96,
    maximumDirectories: 32,
    maximumDepth: 6,
  });
  const actualRecipes = recipeInventory.files.map((path) => `desktop/third_party/mpv/build/${path}`);
  exactStringSet(actualRecipes, lock.recipe_inventory.map((recipe) => recipe.path), '构建配方文件集合');
  for (const recipe of lock.recipe_inventory) {
    const path = resolve(workspaceRootPath, ...recipe.path.split('/'));
    assert(pathWithin(resolve(workspaceRootPath), path), `构建配方路径越界：${recipe.path}`);
    const observed = await inspectAndHashFile(path, recipeInventory.canonicalRoot, recipe.size_bytes);
    assert(observed.size === recipe.size_bytes, `构建配方大小不匹配：${recipe.path}`);
    assert(observed.sha256 === recipe.sha256, `构建配方 SHA-256 不匹配：${recipe.path}`);
  }

  return {
    schemaVersion: 1,
    gate: 'mpv-reproducible-build-inputs',
    checkStatus: 'passed',
    status: 'input_bytes_match_lock',
    metadataComplete: true,
    inputBytesMatchLock: true,
    admitted: false,
    cacheFileCount: inventory.files.length,
    blockers: ['构建尚未执行；供应候选和正式发布均未准入'],
  };
}

async function main() {
  const args = process.argv.slice(2);
  assert(
    args.length <= 1 && (args.length === 0 || args[0] === '--require-verified'),
    '只允许可选参数 --require-verified',
  );
  const report = await runReproducibleBuildInputsGate();
  console.log(JSON.stringify(report, null, 2));
  if (args[0] === '--require-verified' && !report.inputBytesMatchLock) process.exitCode = 2;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
