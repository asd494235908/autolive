import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { verifyAkVirtualCameraLock } from './verify-akvirtualcamera-lock.mjs';

const lockUrl = new URL('../third_party/akvirtualcamera/upstream.lock.json', import.meta.url);
const verifierUrl = new URL('./verify-akvirtualcamera-lock.mjs', import.meta.url);
const verifierPath = fileURLToPath(verifierUrl);

async function repositoryLock() {
  return JSON.parse(await readFile(lockUrl, 'utf8'));
}

function digest(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

async function writeLockedFile(root, declaration, content) {
  const bytes = Buffer.from(content);
  const path = join(root, ...declaration.path.split('/'));
  await mkdir(join(path, '..'), { recursive: true });
  await writeFile(path, bytes);
  declaration.sha256 = digest(bytes);
  if (Object.hasOwn(declaration, 'size_bytes')) declaration.size_bytes = bytes.length;
}

function completeEvidence(role, lock) {
  if (role === 'cyclonedx-sbom') {
    return JSON.stringify({
      bomFormat: 'CycloneDX',
      specVersion: '1.5',
      version: 1,
      components: [{
        type: 'library',
        name: 'akvirtualcamera',
        version: lock.commit,
        licenses: [{ license: { id: 'GPL-3.0-only' } }],
      }],
    });
  }
  if (role === 'legal-review') {
    return '# 法务审查\n\nstatus: approved\nreviewer: Release Counsel\n\n本审查覆盖 GPL-3.0-only。';
  }
  if (role === 'gpu-benchmark-720p30') {
    return JSON.stringify({
      gate: 'akvirtualcamera-gpu-benchmark-720p30',
      status: 'passed',
      requestedSeconds: 7200,
      minimumFrames: 205200,
      framesDelivered: 216000,
      width: 1280,
      height: 720,
      fps: 30,
      timestampMonotonic: true,
      p99ReadbackWithinBudget: true,
      frameCadenceWithinBudget: true,
      gpuScaleAndColorConvert: true,
      zeroCopy: false,
      gpuFacts: {
        adapterLuid: '00000000-0000-0000',
        adapterName: 'Fixture GPU',
        vendorId: 4318,
        deviceId: 1234,
        featureLevel: '0xb000',
      },
      processResourceUsage: {
        sampleCount: 7201,
        initialWorkingSetBytes: 100000000,
        finalWorkingSetBytes: 101000000,
        peakWorkingSetBytes: 102000000,
        workingSetDeltaBytes: 1000000,
        workingSetPeakGrowthBytes: 2000000,
        initialVirtualMemoryBytes: 500000000,
        finalVirtualMemoryBytes: 501000000,
        peakVirtualMemoryBytes: 502000000,
        virtualMemoryDeltaBytes: 1000000,
        virtualMemoryPeakGrowthBytes: 2000000,
        growthWithinBudget: true,
      },
    });
  }
  if (role === 'compatibility-matrix') {
    const ids = [
      'windows-10-directshow', 'windows-11-directshow', 'intel', 'nvidia', 'amd',
      'multi-monitor', 'dpi-100', 'dpi-125', 'dpi-150', 'chrome', 'edge', 'teams',
      'zoom', 'discord', 'obs', 'windows-camera', 'directshow-32bit', 'cleanup',
    ];
    return JSON.stringify({
      schemaVersion: 1,
      status: 'passed',
      entries: ids.map((id) => ({ id, status: 'passed' })),
    });
  }
  return `evidence-${role}`;
}

async function completeFixture() {
  const lock = await repositoryLock();
  const root = await mkdtemp(join(tmpdir(), 'akvirtualcamera-lock-'));
  await writeLockedFile(root, lock.source_archive, 'locked-source-archive');
  for (const artifact of lock.release_requirements.artifacts) {
    await writeLockedFile(root, artifact, `${artifact.role}-${artifact.architecture}`);
    const signature = { path: artifact.signature_path, sha256: artifact.signature_sha256 };
    await writeLockedFile(root, signature, `authenticode-${artifact.role}-${artifact.architecture}`);
    artifact.signature_sha256 = signature.sha256;
  }
  for (const document of lock.release_requirements.documents) {
    await writeLockedFile(root, document, completeEvidence(document.role, lock));
  }
  return { lock, root };
}

test('仓库锁固定上游、许可证、归档哈希和 DirectShow 目标', async () => {
  const lock = await repositoryLock();
  assert.equal(lock.repository, 'https://github.com/webcamoid/akvirtualcamera');
  assert.equal(lock.commit, '9cf77ae6379e5f635255f4b377478d388a46a3b2');
  assert.equal(lock.license_expression, 'GPL-3.0-only');
  assert.equal(lock.source_archive.sha256, '38adfa9eb271f5e602cc26b495c2c7e761b2e707b2bc9bbb047d57ce9c1e231a');
  assert.deepEqual(lock.build.directshow.architectures, ['x86', 'x64']);
  assert.equal(lock.build.media_foundation.policy, 'experimental_not_enabled');
  assert.equal(lock.runtime.zero_copy, false);
  assert.deepEqual(
    lock.release_requirements.artifacts
      .filter(({ role }) => role === 'capi' || role === 'sidecar')
      .map(({ role, architecture }) => [role, architecture]),
    [['capi', 'x64'], ['sidecar', 'x64']],
  );

  const report = await verifyAkVirtualCameraLock(lock);
  assert.equal(report.checkStatus, 'passed');
  assert.equal(report.status, 'blocked');
  assert.equal(report.releaseReady, false);
  assert.equal(report.repository, lock.repository);
  assert.equal(report.commit, lock.commit);
  assert.equal(report.sourceArchiveVerified, true);
  assert.match(report.blockers.join('\n'), /artifact\[0\] directshow-filter\/x86 缺少文件/);
});

test('完整本地归档、产物、签名证据齐全时才允许 release_ready', async () => {
  const { lock, root } = await completeFixture();
  try {
    const report = await verifyAkVirtualCameraLock(lock, { repoRoot: root });
    assert.equal(report.metadataComplete, true);
    assert.equal(report.releaseReady, true);
    assert.equal(report.status, 'release_ready');
    assert.equal(report.sourceArchiveVerified, true);
    assert.equal(report.verifiedArtifactCount, lock.release_requirements.artifacts.length);
    assert.equal(report.verifiedDocumentCount, lock.release_requirements.documents.length);
    assert.deepEqual(report.blockers, []);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('上游身份、归档绑定和许可证漂移均 fail-closed', async () => {
  const cases = [
    ['错误仓库', (lock) => { lock.repository = 'https://github.com/example/akvirtualcamera'; }, /上游仓库/],
    ['移动提交', (lock) => { lock.commit = 'main'; }, /40 位/],
    ['许可证漂移', (lock) => { lock.license_expression = 'GPL-3.0'; }, /GPL-3.0-only/],
    ['归档使用移动 URL', (lock) => { lock.source_archive.url = 'https://github.com/webcamoid/akvirtualcamera/archive/master.tar.gz'; }, /不可变 commit/],
    ['归档哈希格式错误', (lock) => { lock.source_archive.sha256 = 'sha256'; }, /64 位/],
    ['归档大小无效', (lock) => { lock.source_archive.size_bytes = 0; }, /size_bytes/],
    ['DirectShow 架构缺失', (lock) => { lock.build.directshow.architectures = ['x64']; }, /精确匹配/],
    ['Media Foundation 默认开启', (lock) => { lock.build.media_foundation.policy = 'enabled'; }, /实验且默认关闭/],
    ['GPU 零拷贝漂移', (lock) => { lock.runtime.zero_copy = true; }, /zero_copy=false/],
    ['产物路径漂移', (lock) => { lock.release_requirements.artifacts[0].path = 'desktop/other.dll'; }, /产物集合/],
    ['产物哈希格式错误', (lock) => { lock.release_requirements.artifacts[0].sha256 = 'bad'; }, /64 位/],
    ['门禁集合缺失', (lock) => { lock.release_requirements.required_gates.pop(); }, /固定集合/],
  ];
  for (const [name, mutate, pattern] of cases) {
    const lock = await repositoryLock();
    mutate(lock);
    await assert.rejects(() => verifyAkVirtualCameraLock(lock), pattern, name);
  }
});

test('本地归档和签名文件存在但大小或哈希不匹配时阻断', async () => {
  const { lock, root } = await completeFixture();
  try {
    const archivePath = join(root, ...lock.source_archive.path.split('/'));
    await writeFile(archivePath, 'tampered');
    const report = await verifyAkVirtualCameraLock(lock, { repoRoot: root });
    assert.equal(report.releaseReady, false);
    assert.match(report.blockers.join('\n'), /source_archive 字节数不匹配/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('发布证据内容未达到门槛时即使哈希齐全也必须阻断', async () => {
  const { lock, root } = await completeFixture();
  try {
    const benchmark = lock.release_requirements.documents.find(({ role }) => role === 'gpu-benchmark-720p30');
    await writeLockedFile(root, benchmark, JSON.stringify({
      gate: 'akvirtualcamera-gpu-benchmark-720p30',
      status: 'short_smoke_only',
      requestedSeconds: 3,
      width: 1280,
      height: 720,
      fps: 30,
      timestampMonotonic: true,
      p99ReadbackWithinBudget: true,
      gpuScaleAndColorConvert: true,
      zeroCopy: false,
    }));
    const report = await verifyAkVirtualCameraLock(lock, { repoRoot: root });
    assert.equal(report.releaseReady, false);
    assert.match(report.blockers.join('\n'), /gpu-benchmark-720p30 必须标记 status=passed/);
    assert.match(report.blockers.join('\n'), /必须覆盖至少 7200 秒/);
    assert.match(report.blockers.join('\n'), /必须证明帧推进覆盖率和 interframe P95/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('CLI 默认只报告阻断，发布模式以退出码 2 fail-closed', () => {
  const normal = spawnSync(process.execPath, [verifierPath], { encoding: 'utf8' });
  assert.equal(normal.status, 0);
  assert.equal(JSON.parse(normal.stdout).status, 'blocked');

  const release = spawnSync(process.execPath, [verifierPath, '--require-release-ready'], { encoding: 'utf8' });
  assert.equal(release.status, 2);
  assert.equal(JSON.parse(release.stdout).releaseReady, false);

  const unknown = spawnSync(process.execPath, [verifierPath, '--unknown'], { encoding: 'utf8' });
  assert.equal(unknown.status, 1);
  assert.match(unknown.stderr, /--require-release-ready/);
});

test('校验器只读本地锁和文件，不联网或启动外部进程', async () => {
  const source = await readFile(verifierUrl, 'utf8');
  assert.doesNotMatch(source, /node:child_process|\bfetch\s*\(|node:https?|\b(?:exec|spawn)\s*\(/);
});
