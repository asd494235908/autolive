import { readFile, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { runPhase1Gate } from './verify-mpv-phase1.mjs';
import { runReproducibleBuildReportGate } from './verify-mpv-reproducible-build-report.mjs';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const phase7aRoot = join(desktopRoot, 'third_party', 'mpv');
const defaultPaths = Object.freeze({
  lockFilePath: join(phase7aRoot, 'reproducible-build-lock.json'),
  reportFilePath: join(phase7aRoot, 'reproducible-build-report.json'),
  evidenceRoot: join(phase7aRoot, 'build-evidence'),
  buildInputsRoot: join(phase7aRoot, 'build-inputs'),
  ffmpegPath: join(desktopRoot, 'src-tauri', 'binaries', 'ffmpeg.exe'),
  shaderPath: join(desktopRoot, 'src-tauri', 'resources', 'shaders', 'gpu83.hook'),
});

function requiredArtifact(report, role) {
  const matches = report?.artifacts?.filter((artifact) => artifact?.role === role) ?? [];
  if (matches.length !== 1) throw new Error(`Phase 7A 报告缺少唯一产物：${role}`);
  return matches[0];
}

export async function runPhase7bTechnicalGate({
  lockFilePath = defaultPaths.lockFilePath,
  reportFilePath = defaultPaths.reportFilePath,
  evidenceRoot = defaultPaths.evidenceRoot,
  buildInputsRoot = defaultPaths.buildInputsRoot,
  ffmpegPath = defaultPaths.ffmpegPath,
  shaderPath = defaultPaths.shaderPath,
  supplyGate = runReproducibleBuildReportGate,
  phase1Runner = runPhase1Gate,
} = {}) {
  const supply = await supplyGate({
    lockFilePath,
    reportFilePath,
    evidenceRootPath: evidenceRoot,
    buildInputsRootPath: buildInputsRoot,
  });
  if (
    supply?.admitted !== true
    || supply.semanticEvidenceVerified !== true
    || supply.claim !== 'one_locked_cold_build_candidate'
  ) {
    throw new Error('Phase 7A 供应候选未通过语义准入，禁止启动 Phase 7B 媒体矩阵');
  }

  const [lock, report] = await Promise.all([
    readFile(lockFilePath, 'utf8').then(JSON.parse),
    readFile(reportFilePath, 'utf8').then(JSON.parse),
  ]);
  const mpvSource = lock.sources?.find((source) => source?.name === 'mpv');
  if (
    mpvSource?.kind !== 'git'
    || mpvSource.repository !== 'https://github.com/mpv-player/mpv'
    || !/^[a-f0-9]{40}$/.test(mpvSource.commit ?? '')
  ) {
    throw new Error('Phase 7A mpv 源码身份无效');
  }
  const mpv = requiredArtifact(report, 'mpv-executable');
  const spirvCross = requiredArtifact(report, 'spirv-cross-runtime');
  const vulkan = requiredArtifact(report, 'vulkan-loader-runtime');
  for (const artifact of [mpv, spirvCross, vulkan]) {
    if (!/^[a-f0-9]{64}$/.test(artifact.sha256 ?? '')) {
      throw new Error(`Phase 7A 运行产物哈希无效：${artifact.role}`);
    }
  }

  const technical = await phase1Runner({
    paths: {
      ffmpeg: resolve(ffmpegPath),
      mpv: resolve(evidenceRoot, mpv.path),
      shader: resolve(shaderPath),
    },
    mpvIdentity: {
      versionToken: 'v0.41.0-UNKNOWN',
      expectedSha256: mpv.sha256,
      sourceRef: mpvSource.commit,
      sourceUrl: `https://github.com/mpv-player/mpv/tree/${mpvSource.commit}`,
    },
    fullscreenWindow: true,
  });
  return {
    schemaVersion: 1,
    gate: 'mpv-libplacebo-phase7b-technical',
    status: technical.status === 'passed' ? 'passed' : technical.status,
    supply,
    candidateRuntime: {
      mpv: { path: mpv.path, sha256: mpv.sha256 },
      spirvCross: { path: spirvCross.path, sha256: spirvCross.sha256 },
      vulkan: { path: vulkan.path, sha256: vulkan.sha256 },
    },
    technical,
    limitations: ['不代表法律复核已批准', '不代表运行资产已经替换', '不代表 30 分钟长稳通过'],
  };
}

function parseCli(argv) {
  let reportPath = null;
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] !== '--report' || reportPath !== null || !argv[index + 1]) {
      throw new Error(`未知或不完整参数：${argv[index]}`);
    }
    reportPath = resolve(argv[index + 1]);
    index += 1;
  }
  return { reportPath };
}

async function main() {
  const { reportPath } = parseCli(process.argv.slice(2));
  const result = await runPhase7bTechnicalGate();
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (reportPath) await writeFile(reportPath, json);
  process.stdout.write(json);
  if (result.status !== 'passed') process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
