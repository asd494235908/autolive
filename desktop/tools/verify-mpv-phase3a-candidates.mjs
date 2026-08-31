import { access, mkdir, mkdtemp, open, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  CANDIDATE_PATHS,
  RUNTIME_OPTION_NAMES,
  buildPhase3aCandidateContract,
  parseRustGpu83Mappings,
} from './mpv-phase3a-candidate-contract.mjs';
import {
  analyzeScenarioFrames,
  capturePausedScenarioFrames,
  serializeShaderOptions,
} from './mpv-frame-difference.mjs';
import {
  finalReportStatus,
  generateSamples,
  parseCliArgs,
  parseShaderParameters,
  runMpvAttempt,
  verifyShaderContract,
  versionEvidence,
} from './verify-mpv-phase1.mjs';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const repositoryRoot = resolve(desktopRoot, '..');
const DEFAULT_PATHS = Object.freeze({
  ffmpeg: join(desktopRoot, 'src-tauri', 'binaries', 'ffmpeg.exe'),
  mpv: join(desktopRoot, 'src-tauri', 'binaries', 'mpv.exe'),
});
const UPDATE_COUNT = 120;
const MINIMUM_SAMPLE_SPARE_MS = 2_000;
const PHASE3A_SAMPLE_DURATION_SECONDS = 15;
const REAL_MEDIA_CONFIRMATION = '--confirm-real-media-gate';
const PRODUCTION_CAPABILITY_PATH = join(
  desktopRoot, 'src-tauri', 'src', 'media_video_gpu_effects.rs',
);
const PRODUCTION_SHADER_PATH = join(
  desktopRoot, 'src-tauri', 'resources', 'shaders', 'gpu83.hook',
);

export function parsePhase3aCliArgs(argv) {
  let confirmed = false;
  const sharedArguments = [];
  for (const argument of argv) {
    if (argument === REAL_MEDIA_CONFIRMATION) {
      if (confirmed) throw new Error(`${REAL_MEDIA_CONFIRMATION} 不得重复`);
      confirmed = true;
    } else {
      sharedArguments.push(argument);
    }
  }
  return { ...parseCliArgs(sharedArguments), confirmed };
}

export async function reservePhase3aReport(reportPath) {
  await mkdir(dirname(reportPath), { recursive: true });
  return open(reportPath, 'wx');
}

export function phase3aSampleSpec(updateCount = UPDATE_COUNT) {
  if (!Number.isSafeInteger(updateCount) || updateCount < 100) {
    throw new Error('候选 GPU 更新次数不得少于 100');
  }
  const fps = 60;
  return {
    name: '1080p-60fps-phase3a',
    width: 1920,
    height: 1080,
    fps,
    duration: PHASE3A_SAMPLE_DURATION_SECONDS,
    gateRole: 'candidate_full_resolution_render',
  };
}

export function phase3aSampleEvidence(sample, updateCount) {
  if (
    !Number.isSafeInteger(updateCount) || updateCount < 100 ||
    !Number.isSafeInteger(sample?.fps) || sample.fps < 1 ||
    typeof sample?.duration !== 'number' || !Number.isFinite(sample.duration)
  ) throw new Error('Phase 3A 样本证据输入无效');
  const frameCount = sample.duration * sample.fps;
  if (!Number.isSafeInteger(frameCount)) throw new Error('Phase 3A 样本总帧数必须是安全整数');
  if (sample.duration <= MINIMUM_SAMPLE_SPARE_MS / 1_000) throw new Error('Phase 3A 样本不足以保留两秒实测余量');
  return {
    durationSeconds: sample.duration,
    frameCount,
    updateCount,
    minimumSpareDurationSeconds: MINIMUM_SAMPLE_SPARE_MS / 1_000,
    remainingWindowEvidence: 'runtime_pts_and_monotonic_clock',
  };
}

export function deriveProductionCapabilityEvidence(rustSource, shaderSource) {
  const declaredCountMatch = String(rustSource).match(
    /pub const GPU83_PARAMETER_COUNT:\s*usize\s*=\s*([\d_]+)\s*;/,
  );
  const tableMatch = String(rustSource).match(
    /pub static GPU83_PARAMETER_MAPPINGS:[^=]+?=\s*\[([\s\S]*?)\r?\n\];/,
  );
  if (!declaredCountMatch || !tableMatch) throw new Error('无法解析 Rust GPU83 capability 映射表');
  const declaredParameterCount = Number(declaredCountMatch[1].replaceAll('_', ''));
  const mappings = parseRustGpu83Mappings(tableMatch[1]);
  const availableMappings = mappings.filter(({ capability }) => capability === 'AVAILABLE');
  const availableParameterCount = availableMappings.length;
  if (
    !Number.isSafeInteger(declaredParameterCount) || declaredParameterCount <= 0 ||
    mappings.length !== declaredParameterCount ||
    new Set(mappings.map(({ fieldPath }) => fieldPath)).size !== mappings.length ||
    new Set(mappings.map(({ shaderOption }) => shaderOption)).size !== mappings.length
  ) {
    throw new Error(
      `Rust GPU83 capability 映射表不闭合：声明=${declaredParameterCount}，映射=${mappings.length}`,
    );
  }
  const shader = verifyShaderContract(String(shaderSource));
  const shaderOptions = parseShaderParameters(String(shaderSource)).map(({ name }) => name);
  const runtimeOptionSet = new Set(RUNTIME_OPTION_NAMES);
  const productShaderOptions = shaderOptions.filter((name) => !runtimeOptionSet.has(name));
  const runtimeShaderOptions = shaderOptions.filter((name) => runtimeOptionSet.has(name));
  const availableOptions = availableMappings.map(({ shaderOption }) => shaderOption);
  if (
    shader.dynamicParameterCount !== availableParameterCount + RUNTIME_OPTION_NAMES.length ||
    JSON.stringify([...productShaderOptions].sort()) !== JSON.stringify([...availableOptions].sort()) ||
    JSON.stringify([...runtimeShaderOptions].sort()) !== JSON.stringify([...RUNTIME_OPTION_NAMES].sort())
  ) {
    throw new Error(
      `生产 shader 与 Rust AVAILABLE capability 身份不一致：product=${productShaderOptions.join(',')}，runtime=${runtimeShaderOptions.join(',')}，available=${availableOptions.join(',')}`,
    );
  }
  return {
    status: 'matched',
    source: 'Rust GPU83_PARAMETER_MAPPINGS + 已审计生产 gpu83.hook',
    declaredParameterCount,
    mappingParameterCount: mappings.length,
    availableParameterCount,
    unavailableParameterCount: declaredParameterCount - availableParameterCount,
    productionShaderDynamicParameterCount: productShaderOptions.length,
    availableShaderOptions: availableOptions,
    productionShaderSha256: shader.sha256,
    label: `${availableParameterCount}/${declaredParameterCount}`,
  };
}

async function assertRegularFile(path, label) {
  await access(path);
  if (!(await stat(path)).isFile()) throw new Error(`${label} 不是普通文件：${path}`);
}

async function loadCandidateContract() {
  const entries = await Promise.all(CANDIDATE_PATHS.map(async (path) => ({
    path,
    source: await readFile(resolve(repositoryRoot, path), 'utf8'),
  })));
  return buildPhase3aCandidateContract(entries);
}

async function loadProductionCapabilityEvidence() {
  const [rustSource, shaderSource] = await Promise.all([
    readFile(PRODUCTION_CAPABILITY_PATH, 'utf8'),
    readFile(PRODUCTION_SHADER_PATH, 'utf8'),
  ]);
  return deriveProductionCapabilityEvidence(rustSource, shaderSource);
}

function attachFrameDifferenceEvidence(attempt, evidence) {
  const failedFields = evidence.comparisons
    .filter(({ status }) => status !== 'passed')
    .map(({ field }) => field);
  const result = {
    ...attempt,
    sessionProbeEvidence: {
      ...(attempt.sessionProbeEvidence ?? {}),
      pausedSameFrame: true,
    },
    frameDifferenceEvidence: evidence,
  };
  if (failedFields.length > 0) {
    result.status = 'failed';
    result.validationErrors = [
      ...(attempt.validationErrors ?? []),
      `逐字段同帧差异未通过：${failedFields.join(', ')}`,
    ];
    result.error = result.validationErrors.join('；');
  }
  return result;
}

function failAttempt(attempt, message, evidence = {}) {
  const validationErrors = [...(attempt.validationErrors ?? []), message];
  return {
    ...attempt,
    ...evidence,
    status: 'failed',
    validationErrors,
    error: validationErrors.join('；'),
  };
}

async function readPausedPts(ipc, stage) {
  const pts = (await ipc.send(['get_property', 'time-pos/full'])).data;
  if (typeof pts !== 'number' || !Number.isFinite(pts) || pts < 0) {
    throw new Error(`${stage} 缺少有限的暂停媒体 PTS`);
  }
  return pts;
}

function samePath(left, right) {
  return resolve(String(left)).toLowerCase() === resolve(String(right)).toLowerCase();
}

export async function captureDefaultNeutralityFrames({
  ipc,
  directory,
  shaderPaths,
  defaultOptions,
  warmupOptions,
}) {
  if (!Array.isArray(shaderPaths) || shaderPaths.length === 0) throw new Error('候选 shader 列表不能为空');
  if (warmupOptions === undefined) throw new Error('warmupOptions 必填');
  await mkdir(directory);
  const originalPause = (await ipc.send(['get_property', 'pause'])).data === true;
  await ipc.send(['set_property', 'pause', true]);
  try {
    const anchorPtsSeconds = await readPausedPts(ipc, '默认中性参考锚点');
    const initialShaders = (await ipc.send(['get_property', 'glsl-shaders'])).data;
    if (!Array.isArray(initialShaders) || initialShaders.length !== 0) {
      throw new Error('默认中性参考必须从零 shader 会话开始');
    }

    const noShaderPath = join(directory, 'frame-0000.png');
    await ipc.send(['screenshot-to-file', noShaderPath, 'video']);
    const noShaderPts = await readPausedPts(ipc, '无 shader 参考帧');
    if (Math.abs(noShaderPts - anchorPtsSeconds) > 1e-9) throw new Error('无 shader 参考帧跨帧');

    for (const shaderPath of shaderPaths) {
      await ipc.send(['change-list', 'glsl-shaders', 'append', shaderPath]);
    }
    const loadedShaders = (await ipc.send(['get_property', 'glsl-shaders'])).data;
    if (!Array.isArray(loadedShaders)
      || loadedShaders.length !== shaderPaths.length
      || loadedShaders.some((path, index) => !samePath(path, shaderPaths[index]))) {
      throw new Error('动态加载后的 shader 列表与固定九文件契约不一致');
    }

    await ipc.send(['set_property', 'glsl-shader-opts', serializeShaderOptions(warmupOptions)]);
    const warmupPath = join(directory, 'setup-warmup.png');
    await ipc.send(['screenshot-to-file', warmupPath, 'video']);
    const warmupPts = await readPausedPts(ipc, '候选全激活预热帧');
    if (Math.abs(warmupPts - anchorPtsSeconds) > 1e-9) throw new Error('候选全激活预热帧跨帧');

    await ipc.send(['set_property', 'glsl-shader-opts', serializeShaderOptions(defaultOptions)]);
    const candidateDefaultPath = join(directory, 'frame-0001.png');
    await ipc.send(['screenshot-to-file', candidateDefaultPath, 'video']);
    const candidateDefaultPts = await readPausedPts(ipc, '候选默认帧');
    if (Math.abs(candidateDefaultPts - anchorPtsSeconds) > 1e-9) throw new Error('候选默认帧跨帧');

    return {
      anchorPtsSeconds,
      loadedShaderCount: loadedShaders.length,
      setupScreenshotCount: 3,
      warmup: {
        completed: true,
        path: warmupPath,
        mediaPtsSeconds: warmupPts,
      },
      captures: [
        {
          scenarioIndex: 0,
          field: 'candidate_default_neutrality',
          mode: 'expected_same',
          role: 'no_shader',
          path: noShaderPath,
          mediaPtsSeconds: noShaderPts,
        },
        {
          scenarioIndex: 0,
          field: 'candidate_default_neutrality',
          mode: 'expected_same',
          role: 'candidate_default',
          path: candidateDefaultPath,
          mediaPtsSeconds: candidateDefaultPts,
        },
      ],
    };
  } finally {
    await ipc.send(['set_property', 'pause', originalPause]);
  }
}

export async function runPhase3aCandidateGate({ paths = DEFAULT_PATHS, updateCount = UPDATE_COUNT } = {}) {
  if (process.platform !== 'win32') throw new Error('Phase 3A 候选实机门禁只支持 Windows');
  if (!Number.isSafeInteger(updateCount) || updateCount < 100) throw new Error('候选 GPU 更新次数不得少于 100');
  await Promise.all([
    assertRegularFile(paths.ffmpeg, 'ffmpeg'),
    assertRegularFile(paths.mpv, 'mpv'),
  ]);
  const temporaryRoot = await mkdtemp(join(tmpdir(), 'autolive-mpv-phase3a-'));
  const report = {
    schemaVersion: 1,
    gate: 'mpv-libplacebo-phase3a-candidates',
    startedAt: new Date().toISOString(),
    platform: { platform: process.platform, arch: process.arch, node: process.version },
    paths: { ffmpeg: resolve(paths.ffmpeg), mpv: resolve(paths.mpv) },
    updateCount,
    attempts: [],
    status: 'failed',
  };
  try {
    const [contract, productionCapability] = await Promise.all([
      loadCandidateContract(),
      loadProductionCapabilityEvidence(),
    ]);
    const shaderPaths = CANDIDATE_PATHS.map((path) => resolve(repositoryRoot, path));
    report.candidate = {
      contractSha256: contract.sha256,
      counts: contract.counts,
      manifest: contract.manifest,
      productScenarioCount: contract.productScenarios.length,
      logicalScenarioCount: contract.logicalScenarios.length,
      isolationScenarioCount: contract.isolationScenarios.length,
      frameDifferenceScenarioCount: contract.scenarios.length,
      frameDifferenceCaptureCount: contract.scenarios.reduce(
        (count, scenario) => count + scenario.states.length,
        0,
      ),
      runtimeOptionCount: contract.counts.runtime,
      shaderOptionSnapshotCount: contract.counts.snapshot,
      productionPromotion: 'not_performed',
    };
    report.productionCapability = productionCapability;
    report.versions = await versionEvidence(paths);
    const sourceContractMatched = report.versions.mpvSourceContract.status === 'matched';
    const [sample] = await generateSamples(
      paths.ffmpeg,
      temporaryRoot,
      undefined,
      [phase3aSampleSpec(updateCount)],
    );
    if (!sample) throw new Error('缺少固定 1080p60 验收样本');
    report.sample = {
      name: sample.name,
      width: sample.width,
      height: sample.height,
      fps: sample.fps,
      ...phase3aSampleEvidence(sample, updateCount),
      role: 'candidate_compile_field_difference_and_combined_budget',
    };

    for (const backend of ['d3d11', 'vulkan']) {
      const screenshotDirectory = join(temporaryRoot, `screenshots-${backend}`);
      const neutralityDirectory = join(temporaryRoot, `neutrality-${backend}`);
      await mkdir(screenshotDirectory);
      let captures = null;
      let neutrality = null;
      let attempt = await runMpvAttempt({
        mpvPath: paths.mpv,
        shaderPath: '',
        shaderPaths: [],
        sample,
        backend,
        shaderParameters: contract.parameters,
        sourceContractMatched,
        updateCount,
        hiddenWindow: true,
        fullscreenWindow: true,
        allowNoShaderStart: true,
        updateOptionSnapshots: [
          serializeShaderOptions(contract.defaults),
          serializeShaderOptions(contract.combinedActiveOptions),
        ],
        requireUpdatePlaybackContinuity: true,
        sessionSetup: async ({ ipc }) => {
          neutrality = await captureDefaultNeutralityFrames({
            ipc,
            directory: neutralityDirectory,
            shaderPaths,
            defaultOptions: contract.defaults,
            warmupOptions: contract.combinedActiveOptions,
          });
          return {
            anchorPtsSeconds: neutrality.anchorPtsSeconds,
            loadedShaderCount: neutrality.loadedShaderCount,
            warmupCompleted: neutrality.warmup.completed,
            setupScreenshotCount: neutrality.setupScreenshotCount,
            neutralityAnalysisCaptureCount: neutrality.captures.length,
            defaultNeutralityPendingAnalysis: true,
          };
        },
        sessionProbe: async ({ ipc }) => {
          captures = await capturePausedScenarioFrames({
            ipc,
            directory: screenshotDirectory,
            scenarios: contract.scenarios,
          });
          return {
            captureCount: captures.length,
            pausedSameFrame: new Set(captures.map(({ mediaPtsSeconds }) => mediaPtsSeconds)).size === 1,
            anchorPtsSeconds: captures[0]?.mediaPtsSeconds ?? null,
          };
        },
      });
      if (neutrality !== null) {
        try {
          const neutralityEvidence = await analyzeScenarioFrames({
            ffmpegPath: paths.ffmpeg,
            directory: neutralityDirectory,
            captures: neutrality.captures,
          });
          attempt = neutralityEvidence.status === 'passed'
            ? { ...attempt, defaultNeutralityEvidence: neutralityEvidence }
            : failAttempt(attempt, '候选全默认输出与无 shader 参考帧不一致', {
              defaultNeutralityEvidence: neutralityEvidence,
            });
        } catch (error) {
          attempt = failAttempt(attempt, `默认中性帧分析失败：${error.message}`, {
            defaultNeutralityAnalysisError: error.message,
          });
        }
      } else if (attempt.status === 'passed') {
        attempt = failAttempt(attempt, '未生成无 shader/候选全默认中性对照证据');
      }
      if (captures !== null) {
        try {
          const differenceEvidence = await analyzeScenarioFrames({
            ffmpegPath: paths.ffmpeg,
            directory: screenshotDirectory,
            captures,
          });
          attempt = attachFrameDifferenceEvidence(attempt, differenceEvidence);
        } catch (error) {
          attempt = failAttempt(attempt, `逐场景帧差分析失败：${error.message}`, {
            frameDifferenceAnalysisError: error.message,
          });
        }
      } else if (attempt.status === 'passed') {
        attempt = failAttempt(attempt, '未生成逐字段同帧截图证据');
      }
      report.attempts.push(attempt);
    }
    report.status = finalReportStatus(report.attempts);
    const fullyValidated1080p = report.status === 'passed'
      && report.attempts.every(({ resolutionEvidence }) =>
        resolutionEvidence?.windowRenderTarget?.matched === true);
    report.claims = {
      attemptedResolution: '1920x1080',
      maximumValidatedResolution: fullyValidated1080p ? '1920x1080' : null,
      candidateParameterCount: contract.counts.total,
      additionalCandidateParameterCount: contract.counts.additional,
      runtimeOptionCount: contract.counts.runtime,
      shaderOptionSnapshotCount: contract.counts.snapshot,
      allCandidateScenariosPassed:
        report.attempts.every(({ frameDifferenceEvidence }) => frameDifferenceEvidence?.status === 'passed'),
      allDefaultsNeutralAgainstNoShader:
        report.attempts.every(({ defaultNeutralityEvidence }) => defaultNeutralityEvidence?.status === 'passed'),
      combinedP99WithinBudget:
        report.attempts.every(({ frameBudgetEvidence }) => frameBudgetEvidence?.status === 'passed'),
      noRepeatedShaderCompilation:
        report.attempts.every(({ shaderCompilationEvidence }) => shaderCompilationEvidence?.status === 'verified'),
      samePidAndNoNewDrops:
        report.attempts.every(({ pidUnchanged, dropFrameEvidence }) => pidUnchanged && dropFrameEvidence?.status === 'passed'),
      monotonicUpdatePtsWithoutLoop:
        report.attempts.every(({ updatePlaybackTimelineEvidence }) => updatePlaybackTimelineEvidence?.status === 'passed'),
      productionCapabilityAfterGate: productionCapability.label,
    };
    report.finishedAt = new Date().toISOString();
    return report;
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

async function main() {
  const options = parsePhase3aCliArgs(process.argv.slice(2));
  if (options.help) {
    console.log('用法: node desktop/tools/verify-mpv-phase3a-candidates.mjs --confirm-real-media-gate --report <新 json 文件>');
    return;
  }
  if (!options.confirmed || options.report === null) {
    console.error('拒绝启动真实媒体门禁：必须逐次提供 --confirm-real-media-gate 和 --report <新 json 文件>');
    process.exitCode = 2;
    return;
  }
  let reportFile;
  try {
    reportFile = await reservePhase3aReport(options.report);
  } catch (error) {
    console.error(`拒绝启动真实媒体门禁：无法原子占用新报告路径：${error.message}`);
    process.exitCode = 2;
    return;
  }
  let report;
  try {
    report = await runPhase3aCandidateGate();
  } catch (error) {
    report = {
      schemaVersion: 1,
      gate: 'mpv-libplacebo-phase3a-candidates',
      status: /只支持 Windows|ENOENT|not found|无法找到/.test(error.message) ? 'unavailable' : 'failed',
      startedAt: new Date().toISOString(),
      finishedAt: new Date().toISOString(),
      error: error.message,
    };
  }
  const json = `${JSON.stringify(report, null, 2)}\n`;
  try {
    await reportFile.writeFile(json, { encoding: 'utf8' });
  } finally {
    await reportFile.close();
  }
  process.stdout.write(json);
  if (report.status !== 'passed') process.exitCode = 1;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  await main();
}
