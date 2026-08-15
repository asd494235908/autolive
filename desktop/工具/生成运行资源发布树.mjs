import { cpSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
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
export const RESOURCE_BASE_URL = 'http://101.96.208.132:7088/autolive-resources/v0.1.0';

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

function releaseFiles(root, executable, componentRoot = root) {
  const files = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      files.push(...releaseFiles(path, executable, componentRoot));
    } else if (entry.isFile()) {
      files.push({
        relative_path: relative(componentRoot, path).replaceAll('\\', '/'),
        sha256: createHash('sha256').update(readFileSync(path)).digest('hex'),
        size: statSync(path).size,
        executable,
      });
    }
  }
  return files;
}

function copyComponentFiles({ target, sourceRoot, outputRoot }) {
  const releaseRoot = join(outputRoot, 'autolive-resources', RESOURCE_RELEASE);
  const components = [
    { name: 'binaries', scope: target, executable: true },
    { name: 'voice-worker', scope: target, executable: true },
    { name: 'voice-models', scope: 'common', executable: false },
  ];

  rmSync(releaseRoot, { recursive: true, force: true });
  return components.flatMap(({ name, scope, executable }) => {
    const source = join(sourceRoot, name);
    const destination = join(releaseRoot, scope, name);
    cpSync(source, destination, { recursive: true, dereference: true });
    removeReleaseCacheEntries(destination);
    return releaseFiles(destination, executable).map((file) => ({
      ...file,
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
  return { manifestPath, releaseRoot: join(outputRoot, 'autolive-resources', RESOURCE_RELEASE), manifest };
}

function main() {
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
