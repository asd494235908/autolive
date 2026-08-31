import {
  closeSync,
  cpSync,
  existsSync,
  mkdirSync,
  openSync,
  readSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { readDesktopVersion } from './desktop-version.mjs';
import {
  assertExactRegularFileTree,
  commitStagedPaths,
  temporarySiblingPath,
  validatePreparedMpvRuntimeRelease,
  validatePreparedMpvRuntimeFiles,
  WINDOWS_PREPARED_FILES,
  WINDOWS_TARGET,
} from './mpv-runtime-release.mjs';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const supportedTargets = new Set([
  'x86_64-apple-darwin',
  'aarch64-apple-darwin',
  WINDOWS_TARGET,
]);

export const RESOURCE_RELEASE = readDesktopVersion().release;
export const RESOURCE_BASE_URL = `http://101.96.208.132:7088/autolive-resources/${RESOURCE_RELEASE}/`;
export const EMBEDDED_RESOURCE_DIRECTORY = 'embedded-runtime-resources';

const HASH_CHUNK_BYTES = 64 * 1024;
const DEPLOY_INVENTORY = 'autolive-deploy-inventory.json';

function assertSupportedTarget(target) {
  if (!target || !supportedTargets.has(target)) {
    throw new Error(`当前构建平台不受支持：${target ?? '未设置 AUTOLIVE_TARGET_TRIPLE'}`);
  }
}

function detectTargetTriple() {
  if (process.platform === 'darwin') {
    return process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
  }
  if (process.platform === 'win32' && process.arch === 'x64') return WINDOWS_TARGET;
  return null;
}

function removeReleaseExcludedEntries(root) {
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.name.startsWith('.')) {
      rmSync(path, { recursive: true, force: true });
    } else if (entry.isDirectory()) {
      if (['trees', 'blobs'].includes(entry.name)) rmSync(path, { recursive: true, force: true });
      else {
        removeReleaseExcludedEntries(path);
        if (readdirSync(path).length === 0) rmSync(path, { recursive: true, force: true });
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

function sameCopiedFiles(sourceFiles, copiedFiles) {
  const identity = (files) => files
    .map(({ relative_path, size_bytes, sha256 }) => ({ relative_path, size_bytes, sha256 }))
    .sort((left, right) => left.relative_path.localeCompare(right.relative_path));
  if (JSON.stringify(identity(sourceFiles)) !== JSON.stringify(identity(copiedFiles))) {
    throw new Error('Windows 运行资源 staging 的文件集、大小或 SHA-256 与准备树不一致');
  }
}

function copyComponentFiles({ target, sourceRoot, releaseRoot }) {
  const source = join(sourceRoot, 'binaries');
  const destination = join(releaseRoot, target, 'binaries');
  const sourceFiles = target === WINDOWS_TARGET ? releaseFiles(source, true) : null;
  cpSync(source, destination, { recursive: true, dereference: true });
  if (target !== WINDOWS_TARGET) removeReleaseExcludedEntries(destination);
  const copied = releaseFiles(destination, true).map((file) => ({
    ...file,
    executable: !file.relative_path.startsWith('licenses/'),
    component: 'media',
    relative_path: `${target}/binaries/${file.relative_path}`,
  }));
  if (sourceFiles !== null) {
    sameCopiedFiles(
      sourceFiles,
      copied.map((file) => ({
        ...file,
        relative_path: file.relative_path.slice(`${target}/binaries/`.length),
      })),
    );
  }
  return copied;
}

function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
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
  writeJson(join(scopeRoot, DEPLOY_INVENTORY), inventory);
  return inventory;
}

function stageEmbeddedResourceTree({ releaseRoot, embeddedRoot, target }) {
  mkdirSync(join(embeddedRoot, target), { recursive: true });
  cpSync(
    join(releaseRoot, target, 'binaries'),
    join(embeddedRoot, target, 'binaries'),
    { recursive: true, dereference: true },
  );
}

export async function buildRuntimeResourceRelease({
  target = process.env.AUTOLIVE_TARGET_TRIPLE || detectTargetTriple(),
  sourceRoot = join(desktopRoot, 'src-tauri'),
  outputRoot = join(desktopRoot, 'resource-release'),
  manifestPath = join(desktopRoot, 'src-tauri', 'runtime-resources.json'),
  mpvSourceRoot = resolve(
    process.env.AUTOLIVE_MPV_SOURCE_DIR ?? join(desktopRoot, 'third_party', 'mpv'),
  ),
  mpvReleaseGate = validatePreparedMpvRuntimeRelease,
  supplyGate,
} = {}) {
  assertSupportedTarget(target);
  const binariesRoot = join(sourceRoot, 'binaries');
  let mpvRelease = null;
  if (target === WINDOWS_TARGET) {
    assertExactRegularFileTree(binariesRoot, WINDOWS_PREPARED_FILES, 'Windows 准备运行资源树');
    mpvRelease = await mpvReleaseGate({
      binariesRoot,
      mpvSourceRoot,
      target,
      mode: 'release',
      supplyGate,
    });
  }

  const releaseRoot = join(outputRoot, 'autolive-resources', RESOURCE_RELEASE);
  const embeddedRoot = join(sourceRoot, EMBEDDED_RESOURCE_DIRECTORY);
  const releaseStage = temporarySiblingPath(releaseRoot);
  const embeddedStage = temporarySiblingPath(embeddedRoot);
  const manifestStage = temporarySiblingPath(manifestPath);
  try {
    mkdirSync(releaseStage, { recursive: true });
    const files = copyComponentFiles({ target, sourceRoot, releaseRoot: releaseStage });
    if (target === WINDOWS_TARGET) {
      validatePreparedMpvRuntimeFiles({
        binariesRoot: join(releaseStage, target, 'binaries'),
        release: mpvRelease,
        mode: 'release',
      });
    }
    const manifest = {
      schema_version: 1,
      release: RESOURCE_RELEASE,
      target,
      base_url: RESOURCE_BASE_URL,
      files: files.sort((left, right) => left.relative_path.localeCompare(right.relative_path)),
    };
    stageEmbeddedResourceTree({ releaseRoot: releaseStage, embeddedRoot: embeddedStage, target });
    writeDeploymentInventory(join(releaseStage, target), target);
    writeJson(manifestStage, manifest);
    commitStagedPaths([
      { target: releaseRoot, staged: releaseStage },
      { target: embeddedRoot, staged: embeddedStage },
      { target: manifestPath, staged: manifestStage },
    ]);
    return { manifestPath, releaseRoot, embeddedRoot, manifest };
  } finally {
    for (const staged of [releaseStage, embeddedStage, manifestStage]) {
      if (existsSync(staged)) rmSync(staged, { recursive: true, force: true });
    }
  }
}

async function main() {
  const result = await buildRuntimeResourceRelease();
  console.log(`已生成运行资源发布树：${result.releaseRoot}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
