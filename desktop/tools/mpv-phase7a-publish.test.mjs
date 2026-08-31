import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import test from 'node:test';

const exec = promisify(execFile);
const publisher = new URL('../third_party/mpv/build/publish-build-evidence.ps1', import.meta.url);

function digest(value) {
  return createHash('sha256').update(value).digest('hex');
}

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'autolive-phase7a-publish-test-'));
  const mpv = join(root, 'mpv');
  const source = join(root, 'source');
  await mkdir(join(mpv, 'build-evidence'), { recursive: true });
  await mkdir(join(source, 'build-evidence'), { recursive: true });
  await writeFile(join(mpv, 'build-evidence', 'old-only.txt'), 'old');
  await writeFile(join(mpv, 'reproducible-build-report.json'), '{"old":true}\n');
  const bytes = Buffer.from('new evidence');
  await writeFile(join(source, 'build-evidence', 'new.txt'), bytes);
  await writeFile(join(source, 'reproducible-build-report.json'), JSON.stringify({
    artifacts: [{ path: 'new.txt', size: bytes.length, sha256: digest(bytes) }],
  }));
  return { root, mpv, source };
}

async function publish(source, mpv) {
  await exec('pwsh.exe', ['-NoProfile', '-File', publisher.pathname.slice(1), '-SourceRoot', source, '-MpvRoot', mpv]);
}

test('Phase 7A 事务发布完整替换证据目录且不遗留旧文件', async () => {
  const value = await fixture();
  try {
    await publish(value.source, value.mpv);
    assert.deepEqual(await readdir(join(value.mpv, 'build-evidence')), ['new.txt']);
    assert.equal(await readFile(join(value.mpv, 'build-evidence', 'new.txt'), 'utf8'), 'new evidence');
  } finally {
    await rm(value.root, { recursive: true, force: true });
  }
});

test('Phase 7A 发布源复制前校验失败不会破坏旧证据', async () => {
  const value = await fixture();
  try {
    const report = JSON.parse(await readFile(join(value.source, 'reproducible-build-report.json'), 'utf8'));
    report.artifacts[0].sha256 = '0'.repeat(64);
    await writeFile(join(value.source, 'reproducible-build-report.json'), JSON.stringify(report));
    await assert.rejects(() => publish(value.source, value.mpv));
    assert.deepEqual(await readdir(join(value.mpv, 'build-evidence')), ['old-only.txt']);
    assert.equal(await readFile(join(value.mpv, 'reproducible-build-report.json'), 'utf8'), '{"old":true}\n');
  } finally {
    await rm(value.root, { recursive: true, force: true });
  }
});
