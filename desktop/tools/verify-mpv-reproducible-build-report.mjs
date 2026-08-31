import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import { lstat, open, opendir, realpath } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { verifyReproducibleBuildLock } from './verify-mpv-reproducible-build-lock.mjs';
import { verifySemanticBuildEvidence } from './verify-mpv-reproducible-build-evidence.mjs';

const thirdPartyRoot = fileURLToPath(new URL('../third_party/mpv/', import.meta.url));
const defaultLockPath = join(thirdPartyRoot, 'reproducible-build-lock.json');
const defaultReportPath = join(thirdPartyRoot, 'reproducible-build-report.json');
const defaultEvidenceRoot = join(thirdPartyRoot, 'build-evidence');
const defaultBuildInputsRoot = join(thirdPartyRoot, 'build-inputs');
const SHA256 = /^[a-f0-9]{64}$/;
const MAX_EVIDENCE_FILES = 64;
const MAX_EVIDENCE_ENTRIES = 96;
const MAX_EVIDENCE_DIRECTORIES = 32;
const MAX_EVIDENCE_DEPTH = 4;
const MAX_LOCK_BYTES = 4 * 1024 * 1024;
const MAX_REPORT_BYTES = 2 * 1024 * 1024;
const MAX_EVIDENCE_TOTAL_BYTES = 12 * 1024 * 1024 * 1024;
const DEFAULT_ARTIFACT_MAX_BYTES = 128 * 1024 * 1024;
const ARTIFACT_MAX_BYTES = Object.freeze({
  'build-log': 256 * 1024 * 1024,
  'corresponding-source-archive': 8 * 1024 * 1024 * 1024,
  'mpv-executable': 512 * 1024 * 1024,
  'spirv-cross-runtime': 128 * 1024 * 1024,
  'vulkan-loader-runtime': 128 * 1024 * 1024,
});
const EXPECTED_FORMATS = Object.freeze({
  'build-log': 'text',
  'build-parameters': 'json',
  'corresponding-source-archive': 'tar-zst',
  'copyright-inventory': 'text',
  'cyclonedx-sbom': 'cyclonedx-json',
  'dependency-lock': 'json',
  'docker-host-evidence': 'json',
  'license-inventory': 'text',
  'meson-introspection': 'json',
  'mpv-executable': 'pe',
  'spirv-cross-runtime': 'pe',
  'vulkan-loader-runtime': 'pe',
  'patch-bundle': 'tar-zst',
  'pe-imports': 'json',
  'spdx-sbom': 'spdx-json',
});

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function exactKeys(value, expected, path) {
  assert(value && typeof value === 'object' && !Array.isArray(value), `${path} 必须是对象`);
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  assert(
    actual.length === wanted.length && actual.every((key, index) => key === wanted[index]),
    `${path} 字段必须精确匹配固定契约`,
  );
}

function exactArray(value, expected, path) {
  assert(Array.isArray(value), `${path} 必须是数组`);
  assert(
    value.length === expected.length && value.every((item, index) => item === expected[index]),
    `${path} 与输入锁不一致`,
  );
}

function exactStringSet(value, expected, path) {
  assert(Array.isArray(value) && value.every((item) => typeof item === 'string'), `${path} 必须是字符串数组`);
  const actual = [...new Set(value)].sort();
  const wanted = [...expected].sort();
  assert(actual.length === value.length, `${path} 不允许重复项`);
  assert(
    actual.length === wanted.length && actual.every((item, index) => item === wanted[index]),
    `${path} 与输入锁不一致`,
  );
}

function artifactPath(value, path) {
  assert(typeof value === 'string' && value.length > 0, `${path} 必须是非空相对路径`);
  assert(!value.includes('\\') && !value.startsWith('/') && !value.includes(':'), `${path} 必须是相对路径`);
  assert(
    value.split('/').every((segment) => segment && segment !== '.' && segment !== '..'),
    `${path} 包含无效路径段`,
  );
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

function sameFileIdentity(left, right, path) {
  for (const field of ['dev', 'ino', 'size']) {
    assert(left[field] === right[field], `${path} 文件身份发生变化`);
  }
}

export function verifyReproducibleBuildReportMetadata({ lock, lockBytes, report }) {
  const lockGate = verifyReproducibleBuildLock(lock);
  assert(lockGate.metadataComplete, `输入锁元数据未完成：${lockGate.blockers.join('；')}`);
  exactKeys(report, [
    'schema_version', 'scope', 'claim', 'lock_sha256', 'configured_inputs', 'artifacts',
    'dynamic_dependencies', 'build_environment',
  ], '$report');
  assert(report.schema_version === 2, 'report.schema_version 必须为 2');
  assert(report.scope === 'phase7a_supply_candidate', 'report.scope 无效');
  assert(report.claim === 'one_locked_cold_build_candidate', 'report.claim 必须克制为一次锁定冷构建候选');
  assert(report.lock_sha256 === sha256(lockBytes), 'report.lock_sha256 与输入锁文件不一致');

  exactKeys(report.configured_inputs, [
    'meson_arguments', 'spirv_cross_cmake_arguments', 'ffmpeg_arguments', 'cache_inventory',
  ], 'report.configured_inputs');
  exactArray(
    report.configured_inputs.spirv_cross_cmake_arguments,
    lock.build_recipe.spirv_cross_cmake_arguments,
    'report.configured_inputs.spirv_cross_cmake_arguments',
  );
  exactArray(
    report.configured_inputs.meson_arguments,
    lock.build_recipe.meson_arguments,
    'report.configured_inputs.meson_arguments',
  );
  exactArray(
    report.configured_inputs.ffmpeg_arguments,
    lock.build_recipe.ffmpeg_arguments,
    'report.configured_inputs.ffmpeg_arguments',
  );
  exactArray(
    report.configured_inputs.cache_inventory,
    lock.cache_inventory,
    'report.configured_inputs.cache_inventory',
  );

  exactKeys(report.build_environment, [
    'target', 'builder_image', 'network', 'source_mode', 'source_date_epoch',
    'locale', 'timezone', 'path_prefix_map',
  ], 'report.build_environment');
  const expectedEnvironment = {
    target: lock.target,
    builder_image: lock.toolchain.builder_image,
    network: lock.fetch_policy.build_network,
    source_mode: lock.fetch_policy.source_mode,
    source_date_epoch: lock.build_recipe.environment.source_date_epoch,
    locale: lock.build_recipe.environment.locale,
    timezone: lock.build_recipe.environment.timezone,
    path_prefix_map: lock.build_recipe.environment.path_prefix_map,
  };
  for (const [key, value] of Object.entries(expectedEnvironment)) {
    assert(report.build_environment[key] === value, `report.build_environment.${key} 与输入锁不一致`);
  }

  assert(Array.isArray(report.dynamic_dependencies) && new Set(report.dynamic_dependencies).size === report.dynamic_dependencies.length, 'report.dynamic_dependencies 必须是无重复数组');
  assert(report.dynamic_dependencies.every((dll) => lock.output_policy.dynamic_dependencies_allowlist.includes(dll)), 'report.dynamic_dependencies 超出锁定允许列表');
  for (const dll of Object.keys(lock.output_policy.bundled_dynamic_dependencies)) {
    assert(report.dynamic_dependencies.includes(dll), `report.dynamic_dependencies 缺少随包 DLL：${dll}`);
  }
  assert(Array.isArray(report.artifacts), 'report.artifacts 必须是数组');
  exactStringSet(
    report.artifacts.map((artifact) => artifact?.role),
    lock.output_policy.required_roles,
    'report.artifacts roles',
  );
  const paths = new Set();
  let declaredBytes = 0;
  for (const [index, artifact] of report.artifacts.entries()) {
    const path = `report.artifacts[${index}]`;
    exactKeys(artifact, ['role', 'path', 'size', 'sha256', 'format'], path);
    artifactPath(artifact.path, `${path}.path`);
    assert(!paths.has(artifact.path), 'report.artifacts 不允许重复路径');
    paths.add(artifact.path);
    assert(Number.isSafeInteger(artifact.size) && artifact.size > 0, `${path}.size 必须为正整数`);
    const maximumBytes = ARTIFACT_MAX_BYTES[artifact.role] ?? DEFAULT_ARTIFACT_MAX_BYTES;
    assert(artifact.size <= maximumBytes, `${path}.size 超过角色上限`);
    declaredBytes += artifact.size;
    assert(declaredBytes <= MAX_EVIDENCE_TOTAL_BYTES, 'report.artifacts 总字节数超过上限');
    assert(SHA256.test(artifact.sha256), `${path}.sha256 无效`);
    assert(artifact.format === EXPECTED_FORMATS[artifact.role], `${path}.format 与角色不匹配`);
  }
  const expectedPaths = {
    'build-log': 'build-log.txt',
    'build-parameters': 'build-parameters.json',
    'corresponding-source-archive': 'corresponding-source.tar.zst',
    'copyright-inventory': 'copyright-inventory.txt',
    'cyclonedx-sbom': 'sbom.cdx.json',
    'dependency-lock': 'dependency-lock.json',
    'docker-host-evidence': 'docker-host-evidence.json',
    'license-inventory': 'license-inventory.txt',
    'meson-introspection': 'meson-introspection.json',
    'mpv-executable': 'mpv.exe',
    'patch-bundle': 'patch-bundle.tar.zst',
    'pe-imports': 'pe-imports.json',
    'spdx-sbom': 'sbom.spdx.json',
    'spirv-cross-runtime': 'spirv-cross-c-shared.dll',
    'vulkan-loader-runtime': 'vulkan-1.dll',
  };
  for (const artifact of report.artifacts) {
    assert(artifact.path === expectedPaths[artifact.role], `report.artifacts 路径与角色不匹配：${artifact.role}`);
  }

  return {
    schemaVersion: 1,
    gate: 'mpv-reproducible-build-report',
    scope: report.scope,
    lockSha256: report.lock_sha256,
    artifacts: report.artifacts.map((artifact) => ({ ...artifact })),
  };
}

export async function inspectAndHashFile(path, canonicalRoot, maximumBytes = Number.MAX_SAFE_INTEGER) {
  const lexicalStat = await lstat(path, { bigint: true });
  assert(lexicalStat.isFile() && !lexicalStat.isSymbolicLink(), `${path} 必须是普通文件`);
  assert(lexicalStat.size <= BigInt(maximumBytes), `${path} 超过 ${maximumBytes} 字节上限`);
  const handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = await handle.stat({ bigint: true });
    assert(before.isFile(), `${path} 必须是普通文件`);
    sameFileIdentity(lexicalStat, before, path);
    sameFileIdentity(await lstat(path, { bigint: true }), before, path);
    const canonical = await realpath(path);
    assert(pathWithin(canonicalRoot, canonical), '构建证据文件不允许越界');
    const hash = createHash('sha256');
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let position = 0;
    while (true) {
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, position);
      if (bytesRead === 0) break;
      hash.update(buffer.subarray(0, bytesRead));
      position += bytesRead;
    }
    const after = await handle.stat({ bigint: true });
    for (const field of ['dev', 'ino', 'size', 'mtimeNs', 'ctimeNs']) {
      assert(before[field] === after[field], `${path} 在哈希期间发生变化`);
    }
    sameFileIdentity(await lstat(path, { bigint: true }), after, path);
    return { size: Number(after.size), sha256: hash.digest('hex') };
  } finally {
    await handle.close();
  }
}

export async function readBoundedRegularFile(path, maximumBytes, label) {
  const lexicalStat = await lstat(path, { bigint: true });
  assert(lexicalStat.isFile() && !lexicalStat.isSymbolicLink(), `${label} 必须是普通文件`);
  assert(lexicalStat.size <= BigInt(maximumBytes), `${label} 超过 ${maximumBytes} 字节上限`);
  const handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = await handle.stat({ bigint: true });
    sameFileIdentity(lexicalStat, before, label);
    const chunks = [];
    let position = 0;
    while (true) {
      const remaining = maximumBytes - position + 1;
      const buffer = Buffer.allocUnsafe(Math.min(64 * 1024, remaining));
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, position);
      if (bytesRead === 0) break;
      position += bytesRead;
      assert(position <= maximumBytes, `${label} 超过 ${maximumBytes} 字节上限`);
      chunks.push(buffer.subarray(0, bytesRead));
    }
    const after = await handle.stat({ bigint: true });
    sameFileIdentity(before, after, label);
    sameFileIdentity(await lstat(path, { bigint: true }), after, label);
    return Buffer.concat(chunks, position);
  } finally {
    await handle.close();
  }
}

function pathWithin(root, candidate) {
  const child = relative(root, candidate);
  return child !== '' && !child.startsWith('..') && !isAbsolute(child);
}

export async function listRegularFiles(root, {
  maximumFiles = MAX_EVIDENCE_FILES,
  maximumEntries = MAX_EVIDENCE_ENTRIES,
  maximumDirectories = MAX_EVIDENCE_DIRECTORIES,
  maximumDepth = MAX_EVIDENCE_DEPTH,
} = {}) {
  const rootStat = await lstat(root);
  assert(rootStat.isDirectory() && !rootStat.isSymbolicLink(), '构建证据根必须是普通目录');
  const canonicalRoot = await realpath(root);
  const files = [];
  let entryCount = 0;
  let directoryCount = 1;
  async function visit(directory, depth) {
    assert(depth <= maximumDepth, `目录深度不得超过 ${maximumDepth}`);
    const stream = await opendir(directory);
    for await (const entry of stream) {
      entryCount += 1;
      assert(entryCount <= maximumEntries, `目录项不得超过 ${maximumEntries} 个`);
      const absolute = join(directory, entry.name);
      assert(!entry.isSymbolicLink(), '构建证据目录不允许符号链接');
      const canonical = await realpath(absolute);
      assert(pathWithin(canonicalRoot, canonical), '构建证据目录不允许联接点越界');
      if (entry.isDirectory()) {
        directoryCount += 1;
        assert(directoryCount <= maximumDirectories, `目录不得超过 ${maximumDirectories} 个`);
        await visit(absolute, depth + 1);
      } else {
        assert(entry.isFile(), '构建证据目录只允许普通文件和目录');
        const path = relative(root, absolute).replaceAll('\\', '/');
        files.push(path);
        assert(files.length <= maximumFiles, `文件不得超过 ${maximumFiles} 个`);
      }
    }
  }
  await visit(root, 0);
  return { files: files.sort(), canonicalRoot };
}

export async function runReproducibleBuildReportGate({
  lockFilePath = defaultLockPath,
  reportFilePath = defaultReportPath,
  evidenceRootPath = defaultEvidenceRoot,
  buildInputsRootPath = defaultBuildInputsRoot,
  semanticVerifier = verifySemanticBuildEvidence,
} = {}) {
  const lockBytes = await readBoundedRegularFile(lockFilePath, MAX_LOCK_BYTES, '输入锁');
  const lock = JSON.parse(lockBytes.toString('utf8'));
  const lockGate = verifyReproducibleBuildLock(lock);
  let reportBytes;
  try {
    reportBytes = await readBoundedRegularFile(reportFilePath, MAX_REPORT_BYTES, '构建后报告');
  } catch (error) {
    if (error?.code !== 'ENOENT') throw error;
    return {
      schemaVersion: 1,
      gate: 'mpv-reproducible-build-report',
      checkStatus: 'passed',
      status: 'blocked',
      admitted: false,
      fileEvidenceVerified: false,
      semanticEvidenceVerified: false,
      reportPresent: false,
      blockers: [...lockGate.blockers, '缺少 reproducible-build-report.json'],
    };
  }
  const report = JSON.parse(reportBytes.toString('utf8'));
  const metadata = verifyReproducibleBuildReportMetadata({ lock, lockBytes, report });
  const evidenceRoot = resolve(evidenceRootPath);
  const reportDirectory = resolve(dirname(reportFilePath));
  assert(evidenceRoot !== reportDirectory, '构建证据目录必须与报告目录分离');
  const declaredPaths = metadata.artifacts.map((artifact) => artifact.path).sort();
  const evidenceInventory = await listRegularFiles(evidenceRoot);
  const actualPaths = evidenceInventory.files;
  assert(
    declaredPaths.length === actualPaths.length
      && declaredPaths.every((path, index) => path === actualPaths[index]),
    '构建证据目录与 report.artifacts 不完全一致',
  );
  for (const artifact of metadata.artifacts) {
    const path = resolve(evidenceRoot, ...artifact.path.split('/'));
    assert(path.startsWith(`${evidenceRoot}\\`) || path.startsWith(`${evidenceRoot}/`), '构建证据路径越界');
    const observed = await inspectAndHashFile(path, evidenceInventory.canonicalRoot, artifact.size);
    assert(observed.size === artifact.size, `${artifact.path} 大小不匹配`);
    assert(observed.sha256 === artifact.sha256, `${artifact.path} SHA-256 不匹配`);
  }
  const semantic = await semanticVerifier({
    lock,
    lockBytes,
    evidenceRoot,
    buildInputsRoot: resolve(buildInputsRootPath),
  });
  assert(semantic?.semanticEvidenceVerified === true, '语义 verifier 未明确验证通过');
  assert(semantic.claim === 'one_locked_cold_build_candidate', '语义 verifier claim 无效');
  return {
    schemaVersion: 1,
    gate: metadata.gate,
    checkStatus: 'passed',
    status: 'phase7a_supply_candidate',
    admitted: true,
    fileEvidenceVerified: true,
    semanticEvidenceVerified: semantic.semanticEvidenceVerified,
    reportPresent: true,
    scope: metadata.scope,
    lockSha256: metadata.lockSha256,
    artifactCount: metadata.artifacts.length,
    claim: semantic.claim,
    limitations: ['不代表 bit-for-bit 可复现', '不代表 Phase 7B 法律/发布准入', '不代表 1080p 实机验收'],
    blockers: [],
  };
}

async function main() {
  const args = process.argv.slice(2);
  let requireAdmitted = false;
  const options = {};
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === '--require-admitted') requireAdmitted = true;
    else if (argument === '--lock') options.lockFilePath = args[++index];
    else if (argument === '--report') options.reportFilePath = args[++index];
    else if (argument === '--evidence') options.evidenceRootPath = args[++index];
    else if (argument === '--build-inputs') options.buildInputsRootPath = args[++index];
    else throw new Error(`未知参数：${argument}`);
  }
  const report = await runReproducibleBuildReportGate(options);
  console.log(JSON.stringify(report, null, 2));
  if (requireAdmitted && !report.admitted) process.exitCode = 2;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
