import assert from 'node:assert/strict';
import test from 'node:test';

import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

import {
  extractArchive,
  materializeArchiveLinks,
  parseArchiveManifest,
  semanticJsonEqual,
  verifySourceTreeContains,
  verifyDockerHostEvidence,
  verifyPatchApplies,
} from './verify-mpv-reproducible-build-evidence.mjs';

function row(mode, size, tail) {
  return `${mode}  0 root root ${size} Jan 01 00:00 ${tail}`;
}

test('Phase 7A 内容级 JSON 比较忽略对象键顺序但保留数组顺序和值差异', () => {
  assert.equal(
    semanticJsonEqual(
      { path: 'fixture', sha256: 'a', sizeBytes: 1, nested: { z: true, a: false } },
      { nested: { a: false, z: true }, sizeBytes: 1, path: 'fixture', sha256: 'a' },
    ),
    true,
  );
  assert.equal(semanticJsonEqual({ value: 1 }, { value: 2 }), false);
  assert.equal(semanticJsonEqual(['a', 'b'], ['b', 'a']), false);
});

test('Phase 7A 归档预扫描允许仍落在根内的相对 symlink/hardlink', () => {
  const manifest = parseArchiveManifest([
    row('drwxr-xr-x', 0, 'sources/lib/'),
    row('-rw-r--r--', 1, 'sources/target'),
    row('lrwxrwxrwx', 0, 'sources/lib/safe -> ../target'),
    row('-rw-r--r--', 0, 'sources/hard link to sources/target'),
  ].join('\n'), 'safe fixture');
  assert.deepEqual(manifest.map((item) => item.type), ['directory', 'file', 'symlink', 'hardlink']);
});

test('Phase 7A 归档预扫描在解包前拒绝越界 symlink 与 hardlink', () => {
  assert.throws(
    () => parseArchiveManifest(row('lrwxrwxrwx', 0, 'sources/bad -> ../../outside'), 'bad symlink'),
    /链接目标越过归档根/,
  );
  assert.throws(
    () => parseArchiveManifest(row('-rw-r--r--', 0, 'sources/bad link to ../outside'), 'bad hardlink'),
    /链接目标越过归档根/,
  );
  assert.throws(
    () => parseArchiveManifest(row('lrwxrwxrwx', 0, 'sources/bad -> C:/outside'), 'drive symlink'),
    /相对路径/,
  );
});

test('Phase 7A 归档预扫描在解包前拒绝特殊文件及压缩炸弹展开上限', () => {
  assert.throws(
    () => parseArchiveManifest(row('crw-rw-rw-', 0, 'sources/device'), 'device fixture'),
    /禁止特殊成员类型/,
  );
  const limits = { maximumMembers: 4, maximumMemberBytes: 10, maximumExpandedBytes: 12 };
  assert.throws(
    () => parseArchiveManifest([row('-rw-r--r--', 8, 'a'), row('-rw-r--r--', 8, 'b')].join('\n'), 'bomb fixture', limits),
    /总展开字节超过上限/,
  );
  assert.throws(
    () => parseArchiveManifest(row('-rw-r--r--', 11, 'large'), 'member fixture', limits),
    /单成员展开大小超过上限/,
  );
});

test('Phase 7A tar 解包失败会清空已创建的受限临时目录', async () => {
  const temporaryRoot = await mkdtemp(join(tmpdir(), 'phase7a-extract-failure-test-'));
  let calls = 0;
  try {
    await assert.rejects(
      extractArchive('fixture.tar.zst', 'extract failure fixture', {
        temporaryRoot,
        execute: async () => {
          calls += 1;
          if (calls === 1) return { stdout: row('-rw-r--r--', 1, 'safe.txt') };
          throw new Error('simulated tar extraction failure');
        },
      }),
      /simulated tar extraction failure/,
    );
    assert.deepEqual(await readdir(temporaryRoot), []);
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
});

test('Phase 7A Windows 安全内部链接按构建语义物化为普通文件', async () => {
  const fixtureRoot = await mkdtemp(join(tmpdir(), 'phase7a-link-materialize-test-'));
  try {
    await writeFile(join(fixtureRoot, 'AGENTS.md'), 'locked\n');
    await materializeArchiveLinks({
      root: fixtureRoot,
      stripComponents: 1,
      manifest: [{
        type: 'symlink', path: 'source/CLAUDE.md', target: 'AGENTS.md', size: 0,
      }],
    });
    assert.equal(await readFile(join(fixtureRoot, 'CLAUDE.md'), 'utf8'), 'locked\n');
  } finally {
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});

test('Phase 7A 补丁检查隔离父仓库并接受锁定的零上下文 hunk', async () => {
  const fixtureRoot = await mkdtemp(fileURLToPath(new URL('../third_party/mpv/build/phase7a-git-fixture-', import.meta.url)));
  const sourceRoot = join(fixtureRoot, 'source');
  const patchPath = join(fixtureRoot, 'zero-context.patch');
  try {
    await mkdir(sourceRoot);
    await writeFile(join(sourceRoot, 'meson.build'), `${Array.from({ length: 48 }, (_, index) => `line-${index + 1}`).join('\n')}\nbefore\n`);
    await writeFile(patchPath, '--- a/meson.build\n+++ b/meson.build\n@@ -49 +49 @@\n-before\n+after\n');
    await assert.doesNotReject(verifyPatchApplies({ sourceRoot, patchPath, temporaryRoot: fixtureRoot }));
    assert.equal((await readFile(join(sourceRoot, 'meson.build'), 'utf8')).trimEnd().endsWith('after'), true);
  } finally {
    await rm(fixtureRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
  }
});

test('Phase 7A 对应源码完整包含门禁拒绝缺文件和字节漂移', async () => {
  const fixtureRoot = await mkdtemp(join(tmpdir(), 'phase7a-source-tree-test-'));
  const expectedRoot = join(fixtureRoot, 'expected');
  const actualRoot = join(fixtureRoot, 'actual');
  try {
    await mkdir(expectedRoot);
    await mkdir(actualRoot);
    await writeFile(join(expectedRoot, 'same.txt'), 'locked\n');
    await writeFile(join(actualRoot, 'same.txt'), 'locked\n');
    await assert.doesNotReject(verifySourceTreeContains({ actualRoot, expectedRoot, label: 'fixture' }));
    await writeFile(join(actualRoot, 'same.txt'), 'drift\n');
    await assert.rejects(verifySourceTreeContains({ actualRoot, expectedRoot, label: 'fixture' }), /字节不一致/);
    await writeFile(join(actualRoot, 'same.txt'), 'locked\n');
    await writeFile(join(actualRoot, 'extra.txt'), 'extra\n');
    await assert.doesNotReject(verifySourceTreeContains({ actualRoot, expectedRoot, label: 'fixture' }));
    await rm(join(actualRoot, 'same.txt'));
    await assert.rejects(verifySourceTreeContains({ actualRoot, expectedRoot, label: 'fixture' }), /缺少锁定文件/);
  } finally {
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});

test('Phase 7A 对应源码只对非 ASCII 文件名接受同目录唯一内容别名', async () => {
  const fixtureRoot = await mkdtemp(join(tmpdir(), 'phase7a-unicode-source-tree-test-'));
  const expectedRoot = join(fixtureRoot, 'expected');
  const actualRoot = join(fixtureRoot, 'actual');
  try {
    await mkdir(join(expectedRoot, 'tests'), { recursive: true });
    await mkdir(join(actualRoot, 'tests'), { recursive: true });
    await writeFile(join(expectedRoot, 'tests', '🌋.def'), 'locked\n');
    await writeFile(join(actualRoot, 'tests', '__.def'), 'locked\n');
    await assert.doesNotReject(verifySourceTreeContains({ actualRoot, expectedRoot, label: 'unicode fixture' }));
    await writeFile(join(actualRoot, 'tests', 'second.def'), 'locked\n');
    await assert.rejects(
      verifySourceTreeContains({ actualRoot, expectedRoot, label: 'unicode fixture' }),
      /无法唯一对应/,
    );
  } finally {
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});

function dockerEvidenceFixture() {
  const lockBytes = Buffer.from('{"fixture":true}\n');
  const lockSha256 = createHash('sha256').update(lockBytes).digest('hex');
  const runId = '1'.repeat(32);
  return {
    lock: { toolchain: { builder_image: `example.invalid/builder@sha256:${'2'.repeat(64)}` } },
    lockBytes,
    evidence: {
      schemaVersion: 2,
      claim: 'one_locked_cold_build_candidate',
      lockSha256,
      dockerBuild: {
        network: 'none', pull: false, context: 'desktop/third_party/mpv',
        dockerfile: 'desktop/third_party/mpv/build/Dockerfile',
      },
      baseImage: {
        reference: `example.invalid/builder@sha256:${'2'.repeat(64)}`,
        id: `sha256:${'3'.repeat(64)}`,
        repoDigests: [`example.invalid/builder@sha256:${'2'.repeat(64)}`],
      },
      recipeImage: { tag: `autolive-mpv-phase7a:${lockSha256.slice(0, 16)}`, id: `sha256:${'4'.repeat(64)}` },
      workVolume: {
        name: `autolive-phase7a-work-${runId}`,
        driver: 'local',
        scope: 'local',
        createdAt: '2026-08-29T00:00:00Z',
        runId,
        existedBeforeCreate: false,
        retained: true,
        labels: { runId, lockSha256, purpose: 'cold-build-work' },
      },
      container: {
        id: '5'.repeat(64),
        name: `autolive-phase7a-${lockSha256.slice(0, 12)}-${runId.slice(0, 12)}`,
        user: '0:0',
        networkMode: 'none',
        readonlyRootfs: true,
        memoryBytes: 4 * 1024 * 1024 * 1024,
        memorySwapBytes: 4 * 1024 * 1024 * 1024,
        entrypoint: ['/bin/bash', '/recipe/build.sh'],
        command: [null],
        startedAt: '2026-08-29T00:00:01Z',
        finishedAt: '2026-08-29T00:01:00Z',
        exitCode: 0,
        dockerStartExitCode: 0,
        status: 'exited',
        mounts: [
          { type: 'bind', name: null, destination: '/build-inputs', readWrite: false, noCopy: false },
          { type: 'bind', name: null, destination: '/out', readWrite: true, noCopy: false },
          { type: 'volume', name: `autolive-phase7a-work-${runId}`, destination: '/work', readWrite: true, noCopy: true },
        ],
      },
    },
  };
}

test('Phase 7A Docker 语义门禁只接受唯一 nocopy volume work 与限定 bind', () => {
  const fixture = dockerEvidenceFixture();
  assert.doesNotThrow(() => verifyDockerHostEvidence(fixture));
  for (const [mutate, pattern] of [
    [(value) => { value.evidence.workVolume.existedBeforeCreate = true; }, /全新创建/],
    [(value) => { value.evidence.container.user = 'gha'; }, /0:0/],
    [(value) => { delete value.evidence.container.user; }, /0:0/],
    [(value) => { value.evidence.container.memoryBytes = 0; }, /4 GiB/],
    [(value) => { value.evidence.container.memorySwapBytes = 8 * 1024 ** 3; }, /swap/],
    [(value) => { value.evidence.container.mounts[2].type = 'bind'; }, /nocopy/],
    [(value) => { value.evidence.container.mounts[2].noCopy = false; }, /nocopy/],
    [(value) => { value.evidence.container.mounts[0].readWrite = true; }, /只读 bind/],
    [(value) => { value.evidence.container.mounts[1].type = 'volume'; }, /可写 bind/],
    [(value) => { value.evidence.container.entrypoint = ['/bin/sh']; }, /entrypoint/],
    [(value) => { value.evidence.baseImage.repoDigests = []; }, /基础镜像摘要/],
    [(value) => { value.evidence.recipeImage.tag = 'drift'; }, /配方镜像标签/],
  ]) {
    const invalid = dockerEvidenceFixture();
    mutate(invalid);
    assert.throws(() => verifyDockerHostEvidence(invalid), pattern);
  }
});
