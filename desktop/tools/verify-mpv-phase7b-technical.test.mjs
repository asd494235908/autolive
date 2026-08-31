import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { runPhase7bTechnicalGate } from './verify-mpv-phase7b-technical.mjs';

const MPV_COMMIT = '7b8915bc1d04c7e1b61184e00c7fbfaab1911e75';
const MPV_HASH = 'a'.repeat(64);

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'phase7b-technical-'));
  const evidenceRoot = join(root, 'evidence');
  await mkdir(evidenceRoot);
  const lockFilePath = join(root, 'lock.json');
  const reportFilePath = join(root, 'report.json');
  await writeFile(lockFilePath, JSON.stringify({
    sources: [{ name: 'mpv', kind: 'git', repository: 'https://github.com/mpv-player/mpv', commit: MPV_COMMIT }],
  }));
  await writeFile(reportFilePath, JSON.stringify({
    artifacts: [
      { role: 'mpv-executable', path: 'mpv.exe', sha256: MPV_HASH },
      { role: 'spirv-cross-runtime', path: 'spirv-cross-c-shared.dll', sha256: 'b'.repeat(64) },
      { role: 'vulkan-loader-runtime', path: 'vulkan-1.dll', sha256: 'c'.repeat(64) },
    ],
  }));
  return { root, evidenceRoot, lockFilePath, reportFilePath };
}

test('Phase 7B 只把已准入 Phase 7A 身份交给既有 1080p 技术矩阵', async () => {
  const paths = await fixture();
  let received;
  try {
    const result = await runPhase7bTechnicalGate({
      ...paths,
      ffmpegPath: join(paths.root, 'ffmpeg.exe'),
      shaderPath: join(paths.root, 'gpu83.hook'),
      supplyGate: async () => ({
        admitted: true,
        semanticEvidenceVerified: true,
        claim: 'one_locked_cold_build_candidate',
        lockSha256: 'd'.repeat(64),
      }),
      phase1Runner: async (options) => {
        received = options;
        return { status: 'passed', claims: { maximumValidatedResolution: '1920x1080' } };
      },
    });

    assert.equal(result.status, 'passed');
    assert.equal(received.paths.mpv, join(paths.evidenceRoot, 'mpv.exe'));
    assert.equal(received.mpvIdentity.expectedSha256, MPV_HASH);
    assert.equal(received.mpvIdentity.sourceRef, MPV_COMMIT);
    assert.equal(received.mpvIdentity.versionToken, 'v0.41.0-UNKNOWN');
    assert.equal(received.fullscreenWindow, true);
    assert.equal(result.supply.claim, 'one_locked_cold_build_candidate');
  } finally {
    await rm(paths.root, { recursive: true, force: true });
  }
});

test('Phase 7B 供应语义未准入时禁止启动任何媒体矩阵', async () => {
  const paths = await fixture();
  let phase1Called = false;
  try {
    await assert.rejects(
      runPhase7bTechnicalGate({
        ...paths,
        supplyGate: async () => ({ admitted: false, semanticEvidenceVerified: false }),
        phase1Runner: async () => {
          phase1Called = true;
          return { status: 'passed' };
        },
      }),
      /Phase 7A 供应候选未通过语义准入/,
    );
    assert.equal(phase1Called, false);
  } finally {
    await rm(paths.root, { recursive: true, force: true });
  }
});
