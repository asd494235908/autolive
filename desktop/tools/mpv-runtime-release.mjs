import { createHash, randomUUID } from 'node:crypto';
import {
  closeSync,
  existsSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
  renameSync,
  rmSync,
} from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';

import { validateMpvRuntimeManifestV2 } from './verify-mpv-runtime-manifest-v2.mjs';
import { runReproducibleBuildReportGate } from './verify-mpv-reproducible-build-report.mjs';

export const WINDOWS_TARGET = 'x86_64-pc-windows-msvc';

const MPV_RUNTIME_FILES = Object.freeze([
  ['mpv.exe', 'mpv.exe', 'mpv-executable'],
  ['spirv-cross-c-shared.dll', 'spirv-cross-c-shared.dll', 'spirv-cross-runtime'],
  ['vulkan-1.dll', 'vulkan-1.dll', 'vulkan-loader-runtime'],
  ['legal/Copyright.txt', 'licenses/mpv/Copyright.txt', null],
  ['legal/GPL-2.0.txt', 'licenses/mpv/GPL-2.0.txt', null],
  ['legal/LGPL-2.1.txt', 'licenses/mpv/LGPL-2.1.txt', null],
  ['legal/SOURCE.md', 'licenses/mpv/SOURCE.md', null],
  ['legal/THIRD-PARTY-NOTICES.md', 'licenses/mpv/THIRD-PARTY-NOTICES.md', null],
]);
const INTERNAL_MANIFEST_SOURCE = 'legal/mpv-runtime-manifest.json';
const INTERNAL_MANIFEST_PREPARED = 'licenses/mpv/mpv-runtime-manifest.json';
export const WINDOWS_MPV_SOURCE_TO_PREPARED = Object.freeze([
  ...MPV_RUNTIME_FILES.map(([source, prepared]) => [source, prepared]),
  [INTERNAL_MANIFEST_SOURCE, INTERNAL_MANIFEST_PREPARED],
]);
const SOURCE_RUNTIME_FILES = Object.freeze([
  ...MPV_RUNTIME_FILES.map(([source]) => source),
  INTERNAL_MANIFEST_SOURCE,
]);

export const WINDOWS_PREPARED_FILES = Object.freeze([
  'ffmpeg.exe',
  'ffprobe.exe',
  ...WINDOWS_MPV_SOURCE_TO_PREPARED.map(([, prepared]) => prepared),
].sort());

const SUPPLY_ARTIFACTS = Object.freeze({
  corresponding_source: ['corresponding-source-archive', 'corresponding-source.tar.zst'],
  copyright_inventory: ['copyright-inventory', 'copyright-inventory.txt'],
  cyclonedx_sbom: ['cyclonedx-sbom', 'sbom.cdx.json'],
  license_inventory: ['license-inventory', 'license-inventory.txt'],
  spdx_sbom: ['spdx-sbom', 'sbom.spdx.json'],
});
const REQUIRED_COMPONENTS = Object.freeze(['mpv', 'libplacebo', 'ffmpeg']);
const HASH_BUFFER_BYTES = 64 * 1024;
const MAX_JSON_BYTES = 16 * 1024 * 1024;

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function posix(path) {
  return path.replaceAll('\\', '/');
}

function expectedDirectories(files) {
  const directories = new Set();
  for (const file of files) {
    const parts = file.split('/');
    for (let index = 1; index < parts.length; index += 1) {
      directories.add(parts.slice(0, index).join('/'));
    }
  }
  return [...directories].sort();
}

export function assertExactRegularFileTree(root, expectedFiles, label) {
  const canonicalRoot = resolve(root);
  const rootMetadata = lstatSync(canonicalRoot);
  assert(rootMetadata.isDirectory() && !rootMetadata.isSymbolicLink(), `${label} 根必须是普通目录`);
  const expected = [...expectedFiles].sort();
  const expectedFileSet = new Set(expected);
  const expectedDirectorySet = new Set(expectedDirectories(expected));
  const files = [];
  const directories = [];
  function visit(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      const metadata = lstatSync(path);
      assert(!metadata.isSymbolicLink(), `${label} 禁止符号链接或目录联接：${path}`);
      const relativePath = posix(relative(canonicalRoot, path));
      if (metadata.isDirectory()) {
        assert(expectedDirectorySet.has(relativePath), `${label} 目录不在固定白名单：${relativePath}`);
        directories.push(relativePath);
        visit(path);
      } else {
        assert(metadata.isFile(), `${label} 只允许普通文件：${path}`);
        assert(expectedFileSet.has(relativePath), `${label} 文件不在固定白名单：${relativePath}`);
        files.push(relativePath);
      }
    }
  }
  visit(canonicalRoot);
  assert(JSON.stringify(files.sort()) === JSON.stringify(expected),
    `${label} 文件集必须双向严格等于固定白名单`);
  assert(JSON.stringify(directories.sort()) === JSON.stringify(expectedDirectories(expected)),
    `${label} 目录集必须双向严格等于固定白名单`);
  return files;
}

export function fileDescriptor(path, label = path) {
  const before = lstatSync(path, { bigint: true });
  assert(before.isFile() && !before.isSymbolicLink(), `${label} 必须是普通文件`);
  assert(before.size > 0n && before.size <= BigInt(Number.MAX_SAFE_INTEGER), `${label} 大小无效`);
  const descriptor = openSync(path, 'r');
  const buffer = Buffer.allocUnsafe(HASH_BUFFER_BYTES);
  const hash = createHash('sha256');
  try {
    for (;;) {
      const bytesRead = readSync(descriptor, buffer, 0, buffer.length, null);
      if (bytesRead === 0) break;
      hash.update(buffer.subarray(0, bytesRead));
    }
  } finally {
    closeSync(descriptor);
  }
  const after = lstatSync(path, { bigint: true });
  for (const field of ['dev', 'ino', 'size', 'mtimeNs', 'ctimeNs']) {
    assert(before[field] === after[field], `${label} 在哈希期间发生变化`);
  }
  return { size_bytes: Number(after.size), sha256: hash.digest('hex') };
}

function assertDescriptor(actual, expected, label) {
  assert(actual.size_bytes === expected.size_bytes, `${label} 大小与准入身份不一致`);
  assert(actual.sha256 === expected.sha256, `${label} SHA-256 与准入身份不一致`);
}

function readJson(path, label) {
  const metadata = lstatSync(path);
  assert(metadata.isFile() && !metadata.isSymbolicLink(), `${label} 必须是普通文件`);
  assert(metadata.size > 0 && metadata.size <= MAX_JSON_BYTES, `${label} 大小无效`);
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (error) {
    throw new Error(`${label} 不是有效 JSON：${error instanceof Error ? error.message : error}`);
  }
}

function readStableJson(path, label) {
  const identity = fileDescriptor(path, label);
  const value = readJson(path, label);
  assertDescriptor(fileDescriptor(path, label), identity, label);
  return { value, identity };
}

function uniqueBy(values, predicate, label) {
  const matches = values?.filter(predicate) ?? [];
  assert(matches.length === 1, `${label} 必须且只能有一项`);
  return matches[0];
}

function reportArtifact(report, role, expectedPath) {
  const artifact = uniqueBy(report.artifacts, (entry) => entry?.role === role,
    `Phase 7A 报告产物 ${role}`);
  assert(artifact.path === expectedPath, `Phase 7A 报告产物路径无效：${role}`);
  assert(Number.isSafeInteger(artifact.size) && artifact.size > 0,
    `Phase 7A 报告产物大小无效：${role}`);
  assert(/^[0-9a-f]{64}$/.test(artifact.sha256 ?? ''),
    `Phase 7A 报告产物 SHA-256 无效：${role}`);
  return artifact;
}

function artifactDescriptor(artifact) {
  return { size_bytes: artifact.size, sha256: artifact.sha256 };
}

function componentIdentity(lock, name) {
  const source = uniqueBy(lock.sources, (entry) => entry?.name === name, `构建锁组件 ${name}`);
  assert(source.kind === 'git' && /^[0-9a-f]{40}$/.test(source.commit ?? ''),
    `构建锁组件提交无效：${name}`);
  assert(typeof source.license_expression === 'string' && source.license_expression.length > 0,
    `构建锁组件许可证无效：${name}`);
  return { source_ref: source.commit, license_expression: source.license_expression };
}

export async function loadValidatedMpvRuntimeRelease({
  mpvSourceRoot,
  target = WINDOWS_TARGET,
  mode = 'release',
  supplyGate = runReproducibleBuildReportGate,
} = {}) {
  assert(target === WINDOWS_TARGET, `mpv v2 运行资源只支持 ${WINDOWS_TARGET}`);
  assert(mode === 'release', '资源准备/生成只允许 release 模式，technical 阶段禁止晋级');
  const lockFilePath = join(mpvSourceRoot, 'reproducible-build-lock.json');
  const reportFilePath = join(mpvSourceRoot, 'reproducible-build-report.json');
  const evidenceRootPath = join(mpvSourceRoot, 'build-evidence');
  const buildInputsRootPath = join(mpvSourceRoot, 'build-inputs');
  const supply = await supplyGate({
    lockFilePath,
    reportFilePath,
    evidenceRootPath,
    buildInputsRootPath,
  });
  assert(
    supply?.admitted === true
      && supply.semanticEvidenceVerified === true
      && supply.claim === 'one_locked_cold_build_candidate',
    'Phase 7A 供应候选未通过完整语义准入，禁止 Phase 7B 资源晋级',
  );

  const { value: lock, identity: lockIdentity } = readStableJson(lockFilePath, 'Phase 7A 构建锁');
  const { value: report, identity: reportIdentity } = readStableJson(
    reportFilePath,
    'Phase 7A 构建报告',
  );
  assert(lock.target === target, 'Phase 7A 构建锁 target 不匹配');
  assert(report.scope === 'phase7a_supply_candidate', 'Phase 7A 构建报告 scope 无效');
  assert(report.claim === 'one_locked_cold_build_candidate', 'Phase 7A 构建报告 claim 无效');
  assert(report.lock_sha256 === lockIdentity.sha256, 'Phase 7A 构建报告与当前锁身份不一致');

  const targetRoot = join(mpvSourceRoot, target);
  assertExactRegularFileTree(targetRoot, SOURCE_RUNTIME_FILES, 'mpv 候选运行目录');
  const runtimeArtifacts = new Map();
  for (const [source, , role] of MPV_RUNTIME_FILES) {
    if (role === null) continue;
    const artifact = reportArtifact(report, role, source);
    const expected = artifactDescriptor(artifact);
    assertDescriptor(fileDescriptor(join(targetRoot, ...source.split('/')), source), expected, source);
    runtimeArtifacts.set(role, expected);
  }

  const supplyEvidence = {
    report: {
      path: 'reproducible-build-report.json',
      ...reportIdentity,
    },
  };
  for (const [name, [role, path]] of Object.entries(SUPPLY_ARTIFACTS)) {
    const artifact = reportArtifact(report, role, path);
    const expected = artifactDescriptor(artifact);
    const evidencePath = join(evidenceRootPath, ...path.split('/'));
    assertDescriptor(fileDescriptor(evidencePath, `Phase 7A 证据 ${role}`), expected, role);
    supplyEvidence[name] = { path: `build-evidence/${path}`, ...expected };
  }

  const expectedIdentity = {
    target,
    build: {
      lock_sha256: lockIdentity.sha256,
      report_sha256: reportIdentity.sha256,
    },
    components: Object.fromEntries(
      REQUIRED_COMPONENTS.map((name) => [name, componentIdentity(lock, name)]),
    ),
    files: {
      'mpv.exe': runtimeArtifacts.get('mpv-executable'),
      'spirv-cross-c-shared.dll': runtimeArtifacts.get('spirv-cross-runtime'),
      'vulkan-1.dll': runtimeArtifacts.get('vulkan-loader-runtime'),
      ...Object.fromEntries(MPV_RUNTIME_FILES
        .filter(([source]) => source.startsWith('legal/'))
        .map(([source]) => [source, fileDescriptor(join(targetRoot, ...source.split('/')), source)])),
    },
    supply_evidence: supplyEvidence,
  };
  const manifestPath = join(targetRoot, ...INTERNAL_MANIFEST_SOURCE.split('/'));
  const { value: manifest, identity: manifestDescriptor } = readStableJson(
    manifestPath,
    'mpv runtime manifest v2',
  );
  validateMpvRuntimeManifestV2(manifest, { mode, expectedIdentity });
  return { manifest, manifestDescriptor, expectedIdentity, targetRoot, supply };
}

export async function validatePreparedMpvRuntimeRelease({
  binariesRoot,
  mpvSourceRoot,
  target = WINDOWS_TARGET,
  mode = 'release',
  supplyGate = runReproducibleBuildReportGate,
} = {}) {
  const release = await loadValidatedMpvRuntimeRelease({ mpvSourceRoot, target, mode, supplyGate });
  validatePreparedMpvRuntimeFiles({ binariesRoot, release, mode });
  return release;
}

export function validatePreparedMpvRuntimeFiles({ binariesRoot, release, mode = 'release' }) {
  assert(mode === 'release', '资源准备/生成只允许 release 模式，technical 阶段禁止晋级');
  assert(release?.expectedIdentity, '缺少已准入的 mpv expectedIdentity');
  assert(release?.manifestDescriptor, '缺少已准入的 mpv manifest 文件身份');
  assertExactRegularFileTree(binariesRoot, WINDOWS_PREPARED_FILES, 'Windows 准备运行资源树');
  for (const [source, prepared] of MPV_RUNTIME_FILES) {
    assertDescriptor(
      fileDescriptor(join(binariesRoot, ...prepared.split('/')), prepared),
      release.expectedIdentity.files[source],
      prepared,
    );
  }
  const manifestPath = join(binariesRoot, ...INTERNAL_MANIFEST_PREPARED.split('/'));
  const { value: manifest, identity: manifestDescriptor } = readStableJson(
    manifestPath,
    '准备树 mpv runtime manifest v2',
  );
  assertDescriptor(manifestDescriptor, release.manifestDescriptor, '准备树 mpv runtime manifest v2');
  validateMpvRuntimeManifestV2(manifest, { mode, expectedIdentity: release.expectedIdentity });
  for (const name of ['ffmpeg.exe', 'ffprobe.exe']) fileDescriptor(join(binariesRoot, name), name);
  return manifest;
}

export function temporarySiblingPath(target, suffix = 'stage') {
  return `${resolve(target)}.${process.pid}.${randomUUID()}.${suffix}`;
}

function pathsOverlap(left, right) {
  const difference = relative(left, right);
  return difference === '' || (!difference.startsWith('..') && !isAbsolute(difference));
}

export function commitStagedPaths(entries) {
  assert(Array.isArray(entries) && entries.length > 0, '事务提交至少需要一个 staging 项');
  const targets = new Set();
  const states = entries.map(({ target, staged }) => {
    const resolvedTarget = resolve(target);
    const resolvedStaged = resolve(staged);
    const targetKey = process.platform === 'win32' ? resolvedTarget.toLowerCase() : resolvedTarget;
    assert(!targets.has(targetKey), `事务目标重复：${resolvedTarget}`);
    targets.add(targetKey);
    assert(existsSync(resolvedStaged), `暂存 staging 不存在：${resolvedStaged}`);
    return {
      target: resolvedTarget,
      staged: resolvedStaged,
      backup: temporarySiblingPath(resolvedTarget, 'backup'),
      hadTarget: existsSync(resolvedTarget),
      committed: false,
    };
  });
  const transactionPaths = states.flatMap(({ target, staged }) => [target, staged]);
  for (let left = 0; left < transactionPaths.length; left += 1) {
    for (let right = left + 1; right < transactionPaths.length; right += 1) {
      assert(
        !pathsOverlap(transactionPaths[left], transactionPaths[right])
          && !pathsOverlap(transactionPaths[right], transactionPaths[left]),
        `事务路径禁止相同或嵌套：${transactionPaths[left]}；${transactionPaths[right]}`,
      );
    }
  }
  try {
    for (const state of states) {
      mkdirSync(dirname(state.target), { recursive: true });
      if (state.hadTarget) renameSync(state.target, state.backup);
      try {
        renameSync(state.staged, state.target);
        state.committed = true;
      } catch (error) {
        if (state.hadTarget && existsSync(state.backup)) renameSync(state.backup, state.target);
        throw error;
      }
    }
  } catch (error) {
    const rollbackErrors = [];
    for (const state of [...states].reverse()) {
      try {
        if (state.committed && existsSync(state.target)) {
          rmSync(state.target, { recursive: true, force: true });
        }
        if (existsSync(state.backup)) renameSync(state.backup, state.target);
      } catch (rollbackError) {
        rollbackErrors.push(rollbackError);
      }
    }
    if (rollbackErrors.length > 0) {
      throw new AggregateError([error, ...rollbackErrors], '运行资源事务提交失败且回滚不完整');
    }
    throw error;
  }
  for (const state of states) {
    if (existsSync(state.backup)) rmSync(state.backup, { recursive: true, force: true });
  }
}
