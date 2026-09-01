import { readFile, stat } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { relative, resolve } from 'node:path';

const lockUrl = new URL('../third_party/akvirtualcamera/upstream.lock.json', import.meta.url);
const repositoryRoot = resolve(fileURLToPath(new URL('../..', import.meta.url)));

const COMMIT = /^[a-f0-9]{40}$/;
const SHA256 = /^[a-f0-9]{64}$/;
const ARCHITECTURES = Object.freeze(['x86', 'x64']);
const REQUIRED_GATES = Object.freeze([
  'legal_review',
  'corresponding_source',
  'sbom',
  'authenticode',
  'gpu_benchmark_720p30',
  'windows_10_11_directshow_matrix',
]);
const EXPECTED_ARTIFACTS = Object.freeze([
  ['directshow-filter', 'x86', 'desktop/third_party/akvirtualcamera/artifacts/akvirtualcamera-directshow-x86.dll'],
  ['directshow-filter', 'x64', 'desktop/third_party/akvirtualcamera/artifacts/akvirtualcamera-directshow-x64.dll'],
  ['assistant', 'x64', 'desktop/third_party/akvirtualcamera/artifacts/akvirtualcamera-assistant-x64.exe'],
  ['manager', 'x64', 'desktop/third_party/akvirtualcamera/artifacts/akvirtualcamera-manager-x64.exe'],
  ['capi', 'x64', 'desktop/third_party/akvirtualcamera/artifacts/akvirtualcamera-vcam-capi-x64.dll'],
  ['sidecar', 'x64', 'desktop/third_party/akvirtualcamera/artifacts/akvirtualcamera-sidecar-x64.exe'],
  ['installer', 'x64', 'desktop/third_party/akvirtualcamera/artifacts/akvirtualcamera-installer-x64.exe'],
]);
const EXPECTED_DOCUMENTS = Object.freeze([
  ['upstream-license', 'desktop/third_party/akvirtualcamera/COPYING'],
  ['modifications', 'desktop/third_party/akvirtualcamera/MODIFICATIONS.md'],
  ['corresponding-source', 'desktop/third_party/akvirtualcamera/corresponding-source-manifest.json'],
  ['cyclonedx-sbom', 'desktop/third_party/akvirtualcamera/sbom.cdx.json'],
  ['legal-review', 'desktop/third_party/akvirtualcamera/legal-review.md'],
  ['gpu-benchmark-720p30', 'desktop/third_party/akvirtualcamera/gpu-benchmark-720p30.json'],
  ['compatibility-matrix', 'desktop/third_party/akvirtualcamera/compatibility-matrix.json'],
]);

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
    `${path} 必须精确匹配固定集合和顺序`,
  );
}

function exactSet(value, expected, path) {
  assert(Array.isArray(value), `${path} 必须是数组`);
  const actual = [...value].sort();
  const wanted = [...expected].sort();
  assert(
    value.every((item) => typeof item === 'string')
      && new Set(value).size === value.length
      && actual.length === wanted.length
      && actual.every((item, index) => item === wanted[index]),
    `${path} 必须精确匹配固定集合且不得重复`,
  );
}

function relativePath(value, path) {
  assert(typeof value === 'string' && value.length > 0, `${path} 必须是非空相对路径`);
  assert(!value.includes('\\') && !value.startsWith('/') && !value.includes(':'), `${path} 必须使用正斜杠相对路径`);
  assert(value.split('/').every((segment) => segment && segment !== '.' && segment !== '..'), `${path} 包含无效路径段`);
}

function httpsUrl(value, path) {
  assert(typeof value === 'string', `${path} 必须是 HTTPS URL`);
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error(`${path} 必须是有效 HTTPS URL`);
  }
  assert(parsed.protocol === 'https:' && parsed.hostname && !parsed.username && !parsed.password,
    `${path} 必须是无凭证 HTTPS URL`);
  assert(!parsed.search && !parsed.hash, `${path} 不允许查询或片段`);
}

function sha256(value, path, { nullable = false } = {}) {
  if (nullable && value === null) return;
  assert(typeof value === 'string' && SHA256.test(value), `${path} 必须是 64 位小写 SHA-256`);
}

function validateFileDeclaration(value, path, { artifact = false } = {}) {
  const keys = artifact
    ? ['role', 'architecture', 'path', 'sha256', 'signature_path', 'signature_sha256']
    : ['role', 'path', 'sha256'];
  exactKeys(value, keys, path);
  assert(typeof value.role === 'string' && value.role.length > 0, `${path}.role 无效`);
  if (artifact) {
    assert(ARCHITECTURES.includes(value.architecture), `${path}.architecture 无效`);
    relativePath(value.signature_path, `${path}.signature_path`);
    assert(value.signature_path.startsWith('desktop/third_party/akvirtualcamera/artifacts/'),
      `${path}.signature_path 必须位于 AkVirtualCamera artifacts 目录`);
    sha256(value.signature_sha256, `${path}.signature_sha256`, { nullable: true });
  }
  relativePath(value.path, `${path}.path`);
  sha256(value.sha256, `${path}.sha256`, { nullable: true });
  assert(value.sha256 !== undefined, `${path}.sha256 必须显式为哈希或 null`);
}

function validateLock(lock) {
  exactKeys(lock, [
    'schema_version', 'component', 'repository', 'commit', 'license_expression',
    'source_archive', 'build', 'runtime', 'release_requirements',
  ], '$');
  assert(lock.schema_version === 1, 'schema_version 必须为 1');
  assert(lock.component === 'akvirtualcamera', 'component 必须为 akvirtualcamera');
  assert(lock.repository === 'https://github.com/webcamoid/akvirtualcamera', 'repository 不是锁定的上游仓库');
  httpsUrl(lock.repository, 'repository');
  assert(COMMIT.test(lock.commit), 'commit 必须是 40 位小写提交哈希');
  assert(lock.license_expression === 'GPL-3.0-only', '许可证必须固定为 GPL-3.0-only');

  exactKeys(lock.source_archive, ['url', 'path', 'size_bytes', 'sha256'], 'source_archive');
  const expectedArchiveUrl = `${lock.repository}/archive/${lock.commit}.tar.gz`;
  assert(lock.source_archive.url === expectedArchiveUrl, 'source_archive.url 必须绑定不可变 commit 归档');
  httpsUrl(lock.source_archive.url, 'source_archive.url');
  relativePath(lock.source_archive.path, 'source_archive.path');
  assert(lock.source_archive.path.startsWith('desktop/third_party/akvirtualcamera/source/'),
    'source_archive.path 必须位于固定 source 目录');
  assert(Number.isSafeInteger(lock.source_archive.size_bytes)
    && lock.source_archive.size_bytes > 0 && lock.source_archive.size_bytes <= 1024 * 1024 * 1024,
  'source_archive.size_bytes 无效');
  sha256(lock.source_archive.sha256, 'source_archive.sha256');

  exactKeys(lock.build, [
    'platform', 'minimum_windows_build', 'generator', 'compiler', 'network',
    'directshow', 'media_foundation',
  ], 'build');
  assert(lock.build.platform === 'windows', 'build.platform 必须为 windows');
  assert(lock.build.minimum_windows_build === 18362, 'Windows 最低版本必须为 18362');
  assert(lock.build.generator === 'cmake' && lock.build.compiler === 'msvc', '构建工具链必须固定为 CMake/MSVC');
  assert(lock.build.network === 'none', '构建必须断网');
  exactKeys(lock.build.directshow, ['enabled', 'architectures', 'default_endpoint'], 'build.directshow');
  assert(lock.build.directshow.enabled === true && lock.build.directshow.default_endpoint === true,
    'DirectShow 必须启用且为默认端点');
  exactArray(lock.build.directshow.architectures, ARCHITECTURES, 'build.directshow.architectures');
  exactKeys(lock.build.media_foundation, ['policy', 'minimum_windows_major'], 'build.media_foundation');
  assert(lock.build.media_foundation.policy === 'experimental_not_enabled'
    && lock.build.media_foundation.minimum_windows_major === 11,
  'Media Foundation 必须保持 Windows 11 实验且默认关闭');

  exactKeys(lock.runtime, [
    'device_name', 'pixel_format', 'width', 'height', 'fps', 'capture_api',
    'gpu_conversion', 'cpu_boundary', 'transport', 'ipc', 'ipc_scope', 'zero_copy',
  ], 'runtime');
  assert(lock.runtime.device_name === 'GpAutoLive Camera', '设备名必须固定为 GpAutoLive Camera');
  assert(lock.runtime.pixel_format === 'YUY2' && lock.runtime.width === 1280
    && lock.runtime.height === 720 && lock.runtime.fps === 30, '首版输出必须固定为 YUY2 1280x720@30');
  assert(lock.runtime.capture_api === 'windows_graphics_capture' && lock.runtime.gpu_conversion === true,
    '捕获和转换必须走 WGC/GPU');
  assert(lock.runtime.cpu_boundary === 'single_bounded_staging_readback' && lock.runtime.zero_copy === false,
    'CPU 边界必须是一次有界回读且 zero_copy=false');
  assert(lock.runtime.transport === 'akvirtualcamera_raw_frame_sidecar'
    && lock.runtime.ipc === 'windows_named_pipe' && lock.runtime.ipc_scope === 'current_user_only',
  'sidecar IPC 必须为当前用户 ACL 的 Named Pipe');

  exactKeys(lock.release_requirements, ['artifacts', 'documents', 'required_gates', 'media_foundation'], 'release_requirements');
  assert(Array.isArray(lock.release_requirements.artifacts)
    && lock.release_requirements.artifacts.length === EXPECTED_ARTIFACTS.length, 'release_requirements.artifacts 数量无效');
  const artifacts = new Set();
  for (const [index, artifact] of lock.release_requirements.artifacts.entries()) {
    const path = `release_requirements.artifacts[${index}]`;
    validateFileDeclaration(artifact, path, { artifact: true });
    const identity = `${artifact.role}:${artifact.architecture}`;
    assert(!artifacts.has(identity), `${path} 产物身份重复`);
    artifacts.add(identity);
    const expected = EXPECTED_ARTIFACTS[index];
    assert(artifact.role === expected[0] && artifact.architecture === expected[1] && artifact.path === expected[2],
      `${path} 未精确匹配锁定产物集合`);
  }
  exactSet([...artifacts], EXPECTED_ARTIFACTS.map(([role, architecture]) => `${role}:${architecture}`),
    'release_requirements.artifacts');

  assert(Array.isArray(lock.release_requirements.documents)
    && lock.release_requirements.documents.length === EXPECTED_DOCUMENTS.length, 'release_requirements.documents 数量无效');
  for (const [index, document] of lock.release_requirements.documents.entries()) {
    const path = `release_requirements.documents[${index}]`;
    validateFileDeclaration(document, path);
    const expected = EXPECTED_DOCUMENTS[index];
    assert(document.role === expected[0] && document.path === expected[1], `${path} 未精确匹配锁定文档集合`);
    assert(document.path !== 'desktop/third_party/akvirtualcamera/NOTICE.md'
      && document.path !== 'desktop/third_party/akvirtualcamera/BUILD.md',
    `${path} 不得用仓库说明替代上游许可证或发布证据`);
  }
  exactSet(lock.release_requirements.required_gates, REQUIRED_GATES, 'release_requirements.required_gates');
  assert(lock.release_requirements.media_foundation === 'experimental_not_enabled',
    'release_requirements.media_foundation 必须保持实验关闭');
}

function repoPath(root, value, label) {
  relativePath(value, label);
  const full = resolve(root, value);
  const escaped = relative(root, full).replaceAll('\\', '/');
  assert(escaped === value, `${label} 必须位于仓库根目录内`);
  return full;
}

async function inspectLockedFile(root, declaration, label, blockers) {
  const fullPath = repoPath(root, declaration.path, `${label}.path`);
  let details;
  try {
    details = await stat(fullPath);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      blockers.push(`${label} 缺少文件：${declaration.path}`);
      return { present: false, verified: false };
    }
    blockers.push(`${label} 无法读取：${declaration.path}`);
    return { present: false, verified: false };
  }
  if (!details.isFile()) {
    blockers.push(`${label} 不是普通文件：${declaration.path}`);
    return { present: false, verified: false };
  }
  if (declaration.sha256 === null) {
    blockers.push(`${label} 尚未锁定 SHA-256：${declaration.path}`);
    return { present: true, verified: false };
  }
  if (details.size !== declaration.size_bytes && declaration.size_bytes !== undefined) {
    blockers.push(`${label} 字节数不匹配：${declaration.path}`);
    return { present: true, verified: false };
  }
  const bytes = await readFile(fullPath);
  const actual = createHash('sha256').update(bytes).digest('hex');
  if (actual !== declaration.sha256) {
    blockers.push(`${label} SHA-256 不匹配：${declaration.path}`);
    return { present: true, verified: false };
  }
  return { present: true, verified: true };
}

async function inspectArtifact(root, artifact, index, blockers) {
  const label = `artifact[${index}] ${artifact.role}/${artifact.architecture}`;
  const output = await inspectLockedFile(root, artifact, label, blockers);
  const signature = await inspectLockedFile(root, {
    path: artifact.signature_path,
    sha256: artifact.signature_sha256,
  }, `${label} Authenticode evidence`, blockers);
  return { ...output, signature };
}

async function readEvidenceJson(root, declaration, label, blockers) {
  const fullPath = repoPath(root, declaration.path, `${label}.path`);
  try {
    return JSON.parse(await readFile(fullPath, 'utf8'));
  } catch (error) {
    if (error?.code === 'ENOENT') return null;
    blockers.push(`${label} JSON 无法解析：${declaration.path}`);
    return null;
  }
}

function requireEvidence(condition, blockers, message) {
  if (!condition) blockers.push(message);
}

async function inspectEvidenceSemantics(root, lock, blockers) {
  const documents = new Map(lock.release_requirements.documents.map((document) => [document.role, document]));

  const sbomDeclaration = documents.get('cyclonedx-sbom');
  if (sbomDeclaration) {
    const sbom = await readEvidenceJson(root, sbomDeclaration, 'cyclonedx-sbom', blockers);
    if (sbom) {
      requireEvidence(sbom.bomFormat === 'CycloneDX', blockers, 'cyclonedx-sbom 必须声明 bomFormat=CycloneDX');
      requireEvidence(typeof sbom.specVersion === 'string' && sbom.specVersion.length > 0,
        blockers, 'cyclonedx-sbom 缺少 specVersion');
      requireEvidence(Number.isInteger(sbom.version) && sbom.version >= 1,
        blockers, 'cyclonedx-sbom 缺少有效 version');
      requireEvidence(Array.isArray(sbom.components) && sbom.components.length > 0,
        blockers, 'cyclonedx-sbom 必须包含至少一个组件');
      const upstream = sbom.components?.find((component) =>
        component?.name === 'akvirtualcamera' && component?.version === lock.commit);
      requireEvidence(Boolean(upstream), blockers,
        'cyclonedx-sbom 必须包含锁定提交的 akvirtualcamera 组件');
      requireEvidence(upstream?.licenses?.some((entry) => entry?.license?.id === lock.license_expression), blockers,
        'cyclonedx-sbom 的上游组件必须声明锁定许可证');
    }
  }

  const legalDeclaration = documents.get('legal-review');
  if (legalDeclaration) {
    const fullPath = repoPath(root, legalDeclaration.path, 'legal-review.path');
    try {
      const text = await readFile(fullPath, 'utf8');
      requireEvidence(/^status:\s*approved\s*$/mi.test(text), blockers,
        'legal-review 必须由法务明确标记 status: approved');
      requireEvidence(/^reviewer:\s*\S.+$/mi.test(text), blockers,
        'legal-review 必须记录 reviewer');
      requireEvidence(/GPL-3\.0-only/.test(text), blockers,
        'legal-review 必须明确审查 GPL-3.0-only');
    } catch (error) {
      if (error?.code !== 'ENOENT') blockers.push(`legal-review 无法读取：${legalDeclaration.path}`);
    }
  }

  const benchmarkDeclaration = documents.get('gpu-benchmark-720p30');
  if (benchmarkDeclaration) {
    const benchmark = await readEvidenceJson(root, benchmarkDeclaration, 'gpu-benchmark-720p30', blockers);
    if (benchmark) {
      requireEvidence(benchmark.gate === 'akvirtualcamera-gpu-benchmark-720p30', blockers,
        'gpu-benchmark-720p30 gate 不匹配');
      requireEvidence(benchmark.status === 'passed', blockers,
        'gpu-benchmark-720p30 必须标记 status=passed');
      requireEvidence(Number.isInteger(benchmark.requestedSeconds) && benchmark.requestedSeconds >= 7_200,
        blockers, 'gpu-benchmark-720p30 必须覆盖至少 7200 秒');
      requireEvidence(benchmark.width === 1280 && benchmark.height === 720 && benchmark.fps === 30,
        blockers, 'gpu-benchmark-720p30 必须覆盖 YUY2 1280×720@30fps');
      const expectedMinimumFrames = Number.isSafeInteger(benchmark.requestedSeconds)
        ? Math.floor(benchmark.requestedSeconds * benchmark.fps * 0.95)
        : 0;
      requireEvidence(Number.isSafeInteger(benchmark.minimumFrames)
        && benchmark.minimumFrames >= expectedMinimumFrames,
      blockers, 'gpu-benchmark-720p30 缺少至少 95% 的目标帧数门槛');
      requireEvidence(Number.isSafeInteger(benchmark.framesDelivered)
        && benchmark.framesDelivered >= (benchmark.minimumFrames ?? Number.MAX_SAFE_INTEGER),
      blockers, 'gpu-benchmark-720p30 实际帧推进未达到目标帧数');
      requireEvidence(benchmark.frameCadenceWithinBudget === true, blockers,
        'gpu-benchmark-720p30 必须证明帧推进覆盖率和 interframe P95 在预算内');
      requireEvidence(benchmark.timestampMonotonic === true && benchmark.p99ReadbackWithinBudget === true,
        blockers, 'gpu-benchmark-720p30 必须证明时间戳单调且 P99 在单帧预算内');
      requireEvidence(benchmark.gpuScaleAndColorConvert === true && benchmark.zeroCopy === false,
        blockers, 'gpu-benchmark-720p30 必须证明 GPU 转换且 zeroCopy=false');
      const gpuFacts = benchmark.gpuFacts;
      requireEvidence(gpuFacts && typeof gpuFacts === 'object' && !Array.isArray(gpuFacts), blockers,
        'gpu-benchmark-720p30 必须记录实际捕获 GPU 事实');
      if (gpuFacts && typeof gpuFacts === 'object' && !Array.isArray(gpuFacts)) {
        requireEvidence(typeof gpuFacts.adapterLuid === 'string' && gpuFacts.adapterLuid.length > 0,
          blockers, 'gpu-benchmark-720p30 缺少实际 adapter LUID');
        requireEvidence(typeof gpuFacts.adapterName === 'string' && gpuFacts.adapterName.length > 0,
          blockers, 'gpu-benchmark-720p30 缺少实际 adapter 名称');
        requireEvidence(Number.isSafeInteger(gpuFacts.vendorId) && gpuFacts.vendorId > 0,
          blockers, 'gpu-benchmark-720p30 缺少有效 VendorId');
        requireEvidence(Number.isSafeInteger(gpuFacts.deviceId) && gpuFacts.deviceId > 0,
          blockers, 'gpu-benchmark-720p30 缺少有效 DeviceId');
        requireEvidence(typeof gpuFacts.featureLevel === 'string' && gpuFacts.featureLevel.length > 0,
          blockers, 'gpu-benchmark-720p30 缺少实际 Feature Level');
      }
      const resourceUsage = benchmark.processResourceUsage;
      requireEvidence(resourceUsage && typeof resourceUsage === 'object' && !Array.isArray(resourceUsage), blockers,
        'gpu-benchmark-720p30 必须记录进程资源采样');
      if (resourceUsage && typeof resourceUsage === 'object' && !Array.isArray(resourceUsage)) {
        requireEvidence(Number.isSafeInteger(resourceUsage.sampleCount) && resourceUsage.sampleCount >= 2,
          blockers, 'gpu-benchmark-720p30 进程资源采样不足');
        requireEvidence(resourceUsage.growthWithinBudget === true,
          blockers, 'gpu-benchmark-720p30 进程资源增长超出预算或未通过采样门禁');
        requireEvidence(Number.isSafeInteger(resourceUsage.workingSetPeakGrowthBytes)
          && resourceUsage.workingSetPeakGrowthBytes <= 64 * 1024 * 1024,
        blockers, 'gpu-benchmark-720p30 工作集峰值增长超过 64 MiB');
        requireEvidence(Number.isSafeInteger(resourceUsage.virtualMemoryPeakGrowthBytes)
          && resourceUsage.virtualMemoryPeakGrowthBytes <= 256 * 1024 * 1024,
        blockers, 'gpu-benchmark-720p30 虚拟内存峰值增长超过 256 MiB');
      }
    }
  }

  const matrixDeclaration = documents.get('compatibility-matrix');
  if (matrixDeclaration) {
    const matrix = await readEvidenceJson(root, matrixDeclaration, 'compatibility-matrix', blockers);
    if (matrix) {
      requireEvidence(matrix.schemaVersion === 1 && matrix.status === 'passed', blockers,
        'compatibility-matrix 必须声明 schemaVersion=1 且 status=passed');
      const requiredIds = new Set([
        'windows-10-directshow', 'windows-11-directshow', 'intel', 'nvidia', 'amd',
        'multi-monitor', 'dpi-100', 'dpi-125', 'dpi-150', 'chrome', 'edge', 'teams',
        'zoom', 'discord', 'obs', 'windows-camera', 'directshow-32bit', 'cleanup',
      ]);
      const entries = Array.isArray(matrix.entries) ? matrix.entries : [];
      const passedIds = new Set(entries.filter((entry) => entry?.status === 'passed').map((entry) => entry.id));
      for (const id of requiredIds) {
        requireEvidence(passedIds.has(id), blockers,
          `compatibility-matrix 缺少已通过条目：${id}`);
      }
    }
  }
}

export async function verifyAkVirtualCameraLock(lock, { repoRoot = repositoryRoot } = {}) {
  validateLock(lock);
  const blockers = [];
  const sourceArchive = await inspectLockedFile(repoRoot, lock.source_archive, 'source_archive', blockers);
  const artifacts = [];
  for (const [index, artifact] of lock.release_requirements.artifacts.entries()) {
    artifacts.push(await inspectArtifact(repoRoot, artifact, index, blockers));
  }
  const documents = [];
  for (const [index, document] of lock.release_requirements.documents.entries()) {
    documents.push(await inspectLockedFile(repoRoot, document, `document[${index}] ${document.role}`, blockers));
  }
  await inspectEvidenceSemantics(repoRoot, lock, blockers);
  const metadataComplete = lock.release_requirements.artifacts.every((file) =>
    file.sha256 !== null && file.signature_sha256 !== null)
    && lock.release_requirements.documents.every((file) => file.sha256 !== null);
  const releaseReady = metadataComplete && blockers.length === 0;
  return {
    schemaVersion: 1,
    gate: 'akvirtualcamera-release',
    checkStatus: 'passed',
    status: releaseReady ? 'release_ready' : 'blocked',
    metadataComplete,
    releaseReady,
    repository: lock.repository,
    commit: lock.commit,
    licenseExpression: lock.license_expression,
    sourceArchiveVerified: sourceArchive.verified,
    artifactCount: artifacts.length,
    verifiedArtifactCount: artifacts.filter(({ present, verified, signature }) => present && verified && signature.verified).length,
    verifiedDocumentCount: documents.filter(({ verified }) => verified).length,
    blockers,
  };
}

async function main() {
  const args = process.argv.slice(2);
  assert(args.length <= 1 && (args.length === 0 || args[0] === '--require-release-ready'),
    '只允许可选参数 --require-release-ready');
  const lock = JSON.parse(await readFile(lockUrl, 'utf8'));
  const report = await verifyAkVirtualCameraLock(lock);
  console.log(JSON.stringify(report, null, 2));
  if (args[0] === '--require-release-ready' && !report.releaseReady) process.exitCode = 2;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
