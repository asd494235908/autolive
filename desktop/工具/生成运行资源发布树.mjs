import { closeSync, cpSync, mkdirSync, openSync, readSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const supportedTargets = new Set([
  'x86_64-apple-darwin',
  'aarch64-apple-darwin',
  'x86_64-pc-windows-msvc',
]);

export const RESOURCE_RELEASE = 'v0.1.0';
export const RESOURCE_BASE_URL = 'http://101.96.208.132:7088/autolive-resources/v0.1.0/';

const HASH_CHUNK_BYTES = 64 * 1024;
const DEPLOY_INVENTORY = 'autolive-deploy-inventory.json';

function assertSupportedTarget(target) {
  if (!target || !supportedTargets.has(target)) {
    throw new Error(`当前构建平台不受支持：${target ?? '未设置 AUTOLIVE_TARGET_TRIPLE'}`);
  }
}

function removeReleaseCacheEntries(root) {
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      if (['.locks', 'trees', 'blobs'].includes(entry.name)) {
        rmSync(path, { recursive: true, force: true });
      } else {
        removeReleaseCacheEntries(path);
      }
    } else if (entry.isFile() && entry.name.endsWith('.log')) {
      rmSync(path, { force: true });
    }
  }
}

function hashFile(path) {
  const descriptor = openSync(path, 'r');
  const buffer = Buffer.allocUnsafe(HASH_CHUNK_BYTES);
  const hash = createHash('sha256');
  try {
    for (;;) {
      const bytesRead = readSync(descriptor, buffer, 0, buffer.length, null);
      if (bytesRead === 0) return hash.digest('hex');
      hash.update(buffer.subarray(0, bytesRead));
    }
  } finally {
    closeSync(descriptor);
  }
}

function releaseFiles(root, executable, componentRoot = root) {
  const files = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      files.push(...releaseFiles(path, executable, componentRoot));
    } else if (entry.isFile()) {
      files.push({
        relative_path: relative(componentRoot, path).replaceAll('\\', '/'),
        sha256: hashFile(path),
        size_bytes: statSync(path).size,
        executable,
      });
    }
  }
  return files;
}

function copyComponentFiles({ target, sourceRoot, outputRoot }) {
  const releaseRoot = join(outputRoot, 'autolive-resources', RESOURCE_RELEASE);
  const components = [
    { name: 'binaries', scope: target, executable: true, component: 'media' },
    { name: 'voice-worker', scope: target, executable: true, component: 'voice-runtime' },
    { name: 'voice-models', scope: 'common', executable: false, component: 'voice-models' },
  ];

  rmSync(releaseRoot, { recursive: true, force: true });
  return components.flatMap(({ name, scope, executable, component }) => {
    const source = join(sourceRoot, name);
    const destination = join(releaseRoot, scope, name);
    cpSync(source, destination, { recursive: true, dereference: true });
    removeReleaseCacheEntries(destination);
    return releaseFiles(destination, executable).map((file) => ({
      ...file,
      component,
      relative_path: `${scope}/${name}/${file.relative_path}`,
    }));
  });
}

function writeJsonAtomically(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  const temporaryPath = `${path}.${process.pid}.tmp`;
  writeFileSync(temporaryPath, `${JSON.stringify(value, null, 2)}\n`);
  renameSync(temporaryPath, path);
}

function writeDeploymentInventory(scopeRoot, scope) {
  const files = releaseFiles(scopeRoot, false)
    .filter((file) => file.relative_path !== DEPLOY_INVENTORY)
    .map(({ relative_path, sha256, size_bytes }) => ({
      relative_path,
      type: 'regular-file',
      size_bytes,
      sha256,
    }))
    .sort((left, right) => left.relative_path.localeCompare(right.relative_path));
  const inventory = { schema: 1, release: RESOURCE_RELEASE, scope, files };
  writeJsonAtomically(join(scopeRoot, DEPLOY_INVENTORY), inventory);
  return inventory;
}

export function buildRuntimeResourceRelease({
  target = process.env.AUTOLIVE_TARGET_TRIPLE,
  sourceRoot = join(desktopRoot, 'src-tauri'),
  outputRoot = join(desktopRoot, 'resource-release'),
  manifestPath = join(desktopRoot, 'src-tauri', 'runtime-resources.json'),
} = {}) {
  assertSupportedTarget(target);
  const files = copyComponentFiles({ target, sourceRoot, outputRoot });
  const manifest = {
    schema_version: 1,
    release: RESOURCE_RELEASE,
    target,
    base_url: RESOURCE_BASE_URL,
    files: files.sort((left, right) => left.relative_path.localeCompare(right.relative_path)),
  };
  writeJsonAtomically(manifestPath, manifest);
  const releaseRoot = join(outputRoot, 'autolive-resources', RESOURCE_RELEASE);
  writeDeploymentInventory(join(releaseRoot, target), target);
  writeDeploymentInventory(join(releaseRoot, 'common'), 'common');
  return { manifestPath, releaseRoot, manifest };
}

export function buildRuntimeCommonRelease({
  sourceRoot = join(desktopRoot, 'src-tauri'),
  outputRoot = join(desktopRoot, 'resource-release'),
} = {}) {
  const commonRoot = join(outputRoot, 'autolive-resources', RESOURCE_RELEASE, 'common');
  const destination = join(commonRoot, 'voice-models');
  rmSync(commonRoot, { recursive: true, force: true });
  cpSync(join(sourceRoot, 'voice-models'), destination, { recursive: true, dereference: true });
  removeReleaseCacheEntries(destination);
  writeDeploymentInventory(commonRoot, 'common');
  return { commonRoot };
}

function main() {
  if (process.argv.includes('--common-only')) {
    const result = buildRuntimeCommonRelease();
    console.log(`已生成公共运行资源发布树：${result.commonRoot}`);
    return;
  }
  const result = buildRuntimeResourceRelease();
  console.log(`已生成运行资源发布树：${result.releaseRoot}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
