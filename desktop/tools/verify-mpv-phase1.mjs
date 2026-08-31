import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { access, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createConnection } from 'node:net';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const DEFAULT_PATHS = Object.freeze({
  ffmpeg: join(desktopRoot, 'src-tauri', 'binaries', 'ffmpeg.exe'),
  mpv: join(desktopRoot, 'src-tauri', 'binaries', 'mpv.exe'),
  shader: join(desktopRoot, 'src-tauri', 'resources', 'shaders', 'gpu83.hook'),
});
const IPC_PREFIX = String.raw`\\.\pipe\autolive-mpv-phase1-`;
const MAX_IPC_RESPONSE_BYTES = 64 * 1024;
const MAX_RETIRED_REQUEST_IDS = 256;
const MAX_PROCESS_OUTPUT_BYTES = 256 * 1024;
const IPC_DEADLINE_MS = 2_000;
const STARTUP_DEADLINE_MS = 12_000;
const GPU_UPDATE_COUNT = 200;
const CPU4_UPDATE_COUNT = 160;
const UPDATE_SETTLE_MS = 20;
const MIN_FRAME_BUDGET_SAMPLES = 100;
export const UPDATE_CONTINUITY_IPC_COMMAND_COUNT = 5;
const MAX_UPDATE_OPTION_SNAPSHOTS = 256;
const MAX_UPDATE_OPTION_SNAPSHOT_BYTES = 32 * 1024;
const MAX_UPDATE_OPTION_SNAPSHOTS_BYTES = 1024 * 1024;
const CPU4_FILTER =
  '@autolive_cpu4:lavfi=[eq@autolive_cpu4_eq=brightness=0:contrast=1:saturation=1,hue@autolive_cpu4_hue=h=0:s=1]';
const SHADER_ERROR =
  /(?:shader|hook).*(?:disabled|error|failed|invalid)|(?:compile|compilation).*(?:error|failed)|(?:error|failed).*(?:shader|hook)/i;
const CPU4_ERROR =
  /(?:lavfi|vf-command|eq|hue).*(?:disabled|error|failed|invalid|not found|unsupported)|(?:error|failed).*(?:lavfi|eq|hue)/i;
const SHADER_COMPILE =
  /(?:compile|compiled|compiling|shaderc).*(?:shader|hook)|(?:shader|hook).*(?:compile|compiled|compiling|shaderc)/i;
const VERIFIED_MPV_SOURCE_REF = '7b8915bc1d';
const VERIFIED_MPV_VERSION_TOKEN = 'g7b8915bc1';
const VERIFIED_MPV_SHA256 = '09dc5c350a70b536cc91015e7a856f77c933fa9d7f4c41563666e51c998a0243';
const VERIFIED_SHADER_SHA256 = '5b426433152f898eb2174bb84893747767aa77f1eb2b310cd7db1f2c4f0a1b40';
const VERIFIED_SHADER_PARAMETER_COUNT = 80;
const VERIFIED_MPV_SOURCE_URL =
  `https://github.com/mpv-player/mpv/blob/${VERIFIED_MPV_SOURCE_REF}/video/out/vo_gpu_next.c`;
const DEFAULT_MPV_IDENTITY = Object.freeze({
  versionToken: VERIFIED_MPV_VERSION_TOKEN,
  expectedSha256: VERIFIED_MPV_SHA256,
  sourceRef: VERIFIED_MPV_SOURCE_REF,
  sourceUrl: VERIFIED_MPV_SOURCE_URL,
});

export function validateMpvIdentity(identity) {
  if (!identity || typeof identity !== 'object' || Array.isArray(identity)) {
    throw new Error('mpv 身份无效');
  }
  if (typeof identity.versionToken !== 'string' || identity.versionToken.length === 0 || identity.versionToken.length > 128) {
    throw new Error('mpv 身份版本令牌无效');
  }
  if (!/^[a-f0-9]{64}$/.test(identity.expectedSha256 ?? '')) {
    throw new Error('mpv 身份 SHA-256 无效');
  }
  if (!/^[a-f0-9]{10,40}$/.test(identity.sourceRef ?? '')) {
    throw new Error('mpv 身份源码提交无效');
  }
  if (
    typeof identity.sourceUrl !== 'string'
    || !identity.sourceUrl.startsWith('https://github.com/mpv-player/mpv/')
    || !identity.sourceUrl.includes(identity.sourceRef)
  ) {
    throw new Error('mpv 身份源码地址无效');
  }
  return Object.freeze({
    versionToken: identity.versionToken,
    expectedSha256: identity.expectedSha256,
    sourceRef: identity.sourceRef,
    sourceUrl: identity.sourceUrl,
  });
}

export const SAMPLE_SPECS = Object.freeze([
  ...[24, 25, 30, 50, 60].map((fps) =>
    Object.freeze({
      name: `1080p-${fps}fps`, width: 1920, height: 1080, fps, duration: 0.75,
      gateRole: 'full_resolution_render',
    }),
  ),
]);

function sleep(milliseconds) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds));
}

function finiteNumber(value, name, minimum, maximum) {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < minimum || value > maximum) {
    throw new Error(`${name} 必须是 ${minimum}..=${maximum} 的有限数`);
  }
  return value;
}

export function parseCliArgs(argv) {
  const result = { report: null, help: false };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--help' || argument === '-h') {
      result.help = true;
    } else if (argument === '--report') {
      const value = argv[index + 1];
      if (!value || value.startsWith('-')) throw new Error('--report 缺少文件路径');
      result.report = resolve(value);
      index += 1;
    } else {
      throw new Error(`不支持的参数：${argument}`);
    }
  }
  return result;
}

export function managedPipeName(id = randomUUID()) {
  const token = String(id);
  if (!token || token.length > 96 || !/^[a-zA-Z0-9-]+$/.test(token)) throw new Error('命名管道令牌无效');
  return `${IPC_PREFIX}${token}`;
}

export function assertManagedPipe(pipeName) {
  if (
    typeof pipeName !== 'string' ||
    !pipeName.startsWith(IPC_PREFIX) ||
    pipeName.length > IPC_PREFIX.length + 96 ||
    !/^[a-zA-Z0-9-]+$/.test(pipeName.slice(IPC_PREFIX.length))
  ) {
    throw new Error('mpv IPC 必须使用受管 Phase 1 命名管道');
  }
  return pipeName;
}

export function encodeIpcRequest(command, requestId) {
  if (!Number.isSafeInteger(requestId) || requestId <= 0) throw new Error('request_id 无效');
  if (!Array.isArray(command) || command.length === 0) throw new Error('IPC command 无效');
  const line = `${JSON.stringify({ command, request_id: requestId })}\n`;
  if (Buffer.byteLength(line) > 32 * 1024) throw new Error('IPC 请求超过大小上限');
  return line;
}

export function parseIpcResponseLine(line, maxBytes = MAX_IPC_RESPONSE_BYTES) {
  const bytes = Buffer.isBuffer(line) ? line : Buffer.from(line);
  if (bytes.length === 0 || bytes.length > maxBytes) throw new Error('mpv IPC 响应大小无效');
  let value;
  try {
    value = JSON.parse(bytes.toString('utf8'));
  } catch (error) {
    throw new Error(`mpv IPC 响应不是有效 JSON：${error.message}`);
  }
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('mpv IPC 响应必须是对象');
  }
  return value;
}

export function percentile(values, percentileValue) {
  if (!Array.isArray(values) || values.length === 0) return null;
  finiteNumber(percentileValue, 'percentile', 0, 100);
  const sorted = values.map((value) => finiteNumber(value, 'latency', 0, Number.MAX_VALUE)).sort((a, b) => a - b);
  const index = Math.max(0, Math.ceil((percentileValue / 100) * sorted.length) - 1);
  return sorted[index];
}

export function summarizeLatencies(latencies) {
  if (latencies.length === 0) return { count: 0, totalMs: 0, minMs: null, maxMs: null, meanMs: null, p99Ms: null, samplesMs: [] };
  const total = latencies.reduce((sum, value) => sum + value, 0);
  return {
    count: latencies.length,
    totalMs: Number(total.toFixed(3)),
    minMs: Number(Math.min(...latencies).toFixed(3)),
    maxMs: Number(Math.max(...latencies).toFixed(3)),
    meanMs: Number((total / latencies.length).toFixed(3)),
    p99Ms: Number(percentile(latencies, 99).toFixed(3)),
    samplesMs: latencies.map((value) => Number(value.toFixed(3))),
  };
}

function appendedSamples(previous, current) {
  if (!Array.isArray(current) || current.length === 0) return [];
  if (!Array.isArray(previous) || previous.length === 0) return current;
  const maximum = Math.min(previous.length, current.length);
  for (let overlap = maximum; overlap > 0; overlap -= 1) {
    const suffix = previous.slice(previous.length - overlap);
    const prefix = current.slice(0, overlap);
    if (suffix.every((value, index) => value === prefix[index])) return current.slice(overlap);
  }
  return current;
}

function passSamplesByDescription(voPasses) {
  const passes = Array.isArray(voPasses?.fresh) ? voPasses.fresh : [];
  const result = new Map();
  for (const pass of passes) {
    if (typeof pass?.desc !== 'string' || pass.desc.length === 0 || result.has(pass.desc)) return null;
    const values = Array.isArray(pass.samples)
      ? pass.samples.filter((value) => typeof value === 'number' && Number.isFinite(value) && value >= 0)
      : [];
    result.set(pass.desc, values);
  }
  return result;
}

export function observeFrameBudgetSample(voPasses) {
  const passes = Array.isArray(voPasses?.fresh) ? voPasses.fresh : [];
  if (passes.length === 0) return null;
  let totalNs = 0;
  const telemetry = [];
  for (const [index, pass] of passes.entries()) {
    if (
      typeof pass?.desc !== 'string' || pass.desc.length === 0 ||
      typeof pass.last !== 'number' || !Number.isFinite(pass.last) || pass.last < 0 ||
      !Number.isSafeInteger(pass.count) || pass.count < 0 ||
      !Array.isArray(pass.samples) ||
      pass.samples.some((value) => typeof value !== 'number' || !Number.isFinite(value) || value < 0)
    ) return null;
    telemetry.push([index, pass.desc, pass.count, pass.samples]);
    totalNs += pass.last;
  }
  return {
    totalNs,
    telemetryIdentity: JSON.stringify(telemetry),
    passDescriptions: passes.map(({ desc }) => desc),
  };
}

function validTelemetryIdentity(identity) {
  if (typeof identity !== 'string' || identity.length === 0) return false;
  try {
    const passes = JSON.parse(identity);
    return Array.isArray(passes) && passes.length > 0 && JSON.stringify(passes) === identity &&
      passes.every((pass, index) => Array.isArray(pass) && pass.length === 4 &&
        pass[0] === index && typeof pass[1] === 'string' && pass[1].length > 0 &&
        Number.isSafeInteger(pass[2]) && pass[2] >= 0 && Array.isArray(pass[3]) &&
        pass[3].every((value) => typeof value === 'number' && Number.isFinite(value) && value >= 0));
  } catch {
    return false;
  }
}

export function advanceFrameBudgetObservation(previousTelemetryIdentity, observation) {
  if (previousTelemetryIdentity !== null && !validTelemetryIdentity(previousTelemetryIdentity)) {
    throw new Error('上一 vo-passes telemetryIdentity 无效');
  }
  if (observation === null) return { accepted: false, telemetryIdentity: previousTelemetryIdentity };
  if (!validTelemetryIdentity(observation?.telemetryIdentity)) {
    throw new Error('vo-passes telemetryIdentity 无效');
  }
  return {
    accepted: observation.telemetryIdentity !== previousTelemetryIdentity,
    telemetryIdentity: observation.telemetryIdentity,
  };
}

export function frameObservationSettleMs(sample) {
  const fps = finiteNumber(sample?.fps, 'sample fps', 1, 240);
  return Math.max(UPDATE_SETTLE_MS, Math.ceil(1_000 / fps) + 5);
}

export function buildFrameBudgetEvidence(
  sample,
  voPasses,
  previousVoPasses = null,
  minimumSamples = MIN_FRAME_BUDGET_SAMPLES,
  observedFrameTotalsNs = null,
) {
  const frameIntervalBudgetMs = Number((1_000 / sample.fps).toFixed(3));
  const gpuProcessingP99TargetMs = sample.fps === 60 ? 12 : frameIntervalBudgetMs;
  if (!Number.isSafeInteger(minimumSamples) || minimumSamples <= 0) throw new Error('帧预算最小样本数无效');
  if (observedFrameTotalsNs !== null) {
    if (
      !Array.isArray(observedFrameTotalsNs) ||
      observedFrameTotalsNs.some((value) => typeof value !== 'number' || !Number.isFinite(value) || value < 0)
    ) throw new Error('逐帧 GPU pass 总耗时样本无效');
    const count = observedFrameTotalsNs.length;
    if (count < minimumSamples) {
      return {
        status: 'unverified', sourceFps: sample.fps, frameIntervalBudgetMs,
        gpuProcessingP99TargetMs, sampleCount: count, minimumSamples,
        reason: `参数更新期间仅采集到 ${count} 份按源帧间隔分隔且相邻不重复的有效 vo-passes 统计快照，少于门禁要求的 ${minimumSamples} 份`,
      };
    }
    const timing = summarizeLatencies(observedFrameTotalsNs.map((value) => value / 1_000_000));
    return {
      status: timing.p99Ms <= gpuProcessingP99TargetMs ? 'passed' : 'failed',
      source: 'mpv vo-passes.fresh last totals from adjacent distinct telemetry snapshots during parameter updates',
      sourceFps: sample.fps,
      frameIntervalBudgetMs,
      gpuProcessingP99TargetMs,
      sampleCount: count,
      minimumSamples,
      timing,
    };
  }
  const currentByDescription = passSamplesByDescription(voPasses);
  const previousByDescription = passSamplesByDescription(previousVoPasses);
  if (!currentByDescription || !previousByDescription || currentByDescription.size === 0) {
    return {
      status: 'unverified', sourceFps: sample.fps, frameIntervalBudgetMs,
      gpuProcessingP99TargetMs, sampleCount: 0, minimumSamples,
      reason: 'mpv vo-passes 缺少可按 desc 唯一匹配的逐帧 GPU pass 样本',
    };
  }
  const descriptions = [...currentByDescription.keys()];
  if (descriptions.some((description) => !previousByDescription.has(description))) {
    return {
      status: 'unverified', sourceFps: sample.fps, frameIntervalBudgetMs,
      gpuProcessingP99TargetMs, sampleCount: 0, minimumSamples,
      reason: 'mpv vo-passes 更新前后无法按 desc 匹配全部活动 pass',
    };
  }
  const samples = descriptions.map((description) =>
    appendedSamples(previousByDescription.get(description), currentByDescription.get(description)),
  );
  const count = samples.length > 0 ? Math.min(...samples.map((values) => values.length)) : 0;
  if (count < minimumSamples) {
    return {
      status: 'unverified', sourceFps: sample.fps, frameIntervalBudgetMs,
      gpuProcessingP99TargetMs, sampleCount: count, minimumSamples,
      reason: `mpv vo-passes 仅得到 ${count} 帧，少于门禁要求的 ${minimumSamples} 帧`,
    };
  }
  const totalsMs = Array.from({ length: count }, (_, index) =>
    samples.reduce((total, values) => total + values[values.length - count + index], 0) / 1_000_000,
  );
  const timing = summarizeLatencies(totalsMs);
  return {
    status: timing.p99Ms <= gpuProcessingP99TargetMs ? 'passed' : 'failed',
    source: 'mpv vo-passes.fresh samples added during parameter updates',
    sourceFps: sample.fps,
    frameIntervalBudgetMs,
    gpuProcessingP99TargetMs,
    passCount: descriptions.length,
    passDescriptions: descriptions,
    sampleCount: count,
    minimumSamples,
    timing,
  };
}

export function buildDropFrameEvidence(before, after) {
  const fields = ['frameDropCount', 'decoderFrameDropCount'];
  const valid = fields.every((field) =>
    Number.isSafeInteger(before?.[field]) && before[field] >= 0 &&
    Number.isSafeInteger(after?.[field]) && after[field] >= before[field],
  );
  if (!valid) {
    return { status: 'unverified', before, after, reason: 'mpv 丢帧计数不可用或不单调' };
  }
  const delta = Object.fromEntries(fields.map((field) => [field, after[field] - before[field]]));
  return {
    status: fields.every((field) => delta[field] === 0) ? 'passed' : 'failed',
    before,
    after,
    delta,
  };
}

export function buildDropFrameTimelineEvidence(samples) {
  const fields = ['frameDropCount', 'decoderFrameDropCount'];
  if (
    !Array.isArray(samples) || samples.length < 2 ||
    samples.some((sample) => fields.some((field) => !Number.isSafeInteger(sample?.[field]) || sample[field] < 0))
  ) {
    return { status: 'unverified', samples, reason: 'mpv 丢帧计数时间线缺失或含非法值' };
  }
  const increments = Object.fromEntries(fields.map((field) => [field, 0]));
  const resets = Object.fromEntries(fields.map((field) => [field, 0]));
  const resetEvents = [];
  for (let index = 1; index < samples.length; index += 1) {
    for (const field of fields) {
      const previous = samples[index - 1][field];
      const current = samples[index][field];
      if (current < previous) {
        resets[field] += 1;
        resetEvents.push({ field, sampleIndex: index, previous, current });
      }
      else increments[field] += current - previous;
    }
  }
  return {
    status: fields.some((field) => increments[field] > 0) || resetEvents.length > 0 ? 'failed' : 'passed',
    sampleCount: samples.length,
    increments,
    resets,
    resetEvents,
    before: samples[0],
    after: samples.at(-1),
  };
}

export function buildUpdatePlaybackTimelineEvidence(samples, updateCount, sample) {
  const expectedSampleCount = Number.isSafeInteger(updateCount) && updateCount > 0
    ? updateCount + 1
    : null;
  const fps = sample?.fps;
  const sampleDurationSeconds = sample?.duration;
  if (
    expectedSampleCount === null || !Array.isArray(samples) ||
    samples.length !== expectedSampleCount ||
    !Number.isSafeInteger(fps) || fps < 1 ||
    typeof sampleDurationSeconds !== 'number' || !Number.isFinite(sampleDurationSeconds) ||
    samples.some(({ mediaPtsSeconds, observedAtMs } = {}) =>
      typeof mediaPtsSeconds !== 'number' || !Number.isFinite(mediaPtsSeconds) || mediaPtsSeconds < 0 ||
      typeof observedAtMs !== 'number' || !Number.isFinite(observedAtMs) || observedAtMs < 0)
  ) {
    return {
      status: 'failed',
      sampleCount: Array.isArray(samples) ? samples.length : 0,
      expectedSampleCount,
      reason: '参数更新阶段 PTS 时间线缺失、数量不符或含非法值',
    };
  }
  const resetEvents = [];
  const stagnationEvents = [];
  const excessivePauseEvents = [];
  const maximumNewPauseSeconds = 1 / fps;
  for (let index = 1; index < samples.length; index += 1) {
    const previous = samples[index - 1];
    const current = samples[index];
    const mediaDeltaSeconds = current.mediaPtsSeconds - previous.mediaPtsSeconds;
    const wallDeltaSeconds = (current.observedAtMs - previous.observedAtMs) / 1_000;
    if (wallDeltaSeconds <= 0) {
      return {
        status: 'failed',
        sampleCount: samples.length,
        expectedSampleCount,
        reason: '参数更新阶段单调时钟未严格推进',
      };
    }
    if (mediaDeltaSeconds < 0) {
      resetEvents.push({
        sampleIndex: index,
        previous: previous.mediaPtsSeconds,
        current: current.mediaPtsSeconds,
      });
    } else if (mediaDeltaSeconds === 0) {
      stagnationEvents.push({ sampleIndex: index, mediaPtsSeconds: current.mediaPtsSeconds, wallDeltaSeconds });
    }
    const newPauseSeconds = wallDeltaSeconds - Math.max(mediaDeltaSeconds, 0);
    if (newPauseSeconds > maximumNewPauseSeconds + 0.001) {
      excessivePauseEvents.push({ sampleIndex: index, mediaDeltaSeconds, wallDeltaSeconds, newPauseSeconds });
    }
  }
  const before = samples[0].mediaPtsSeconds;
  const after = samples.at(-1).mediaPtsSeconds;
  const advanced = after > before;
  const remainingDurationSeconds = sampleDurationSeconds - after;
  const remainingWindowPassed = remainingDurationSeconds >= 2;
  const passed = resetEvents.length === 0 && stagnationEvents.length === 0 &&
    excessivePauseEvents.length === 0 && advanced && remainingWindowPassed;
  return {
    status: passed ? 'passed' : 'failed',
    sampleCount: samples.length,
    expectedSampleCount,
    before,
    after,
    deltaSeconds: Number((after - before).toFixed(9)),
    monotonicNonDecreasing: resetEvents.length === 0,
    strictlyAdvancing: stagnationEvents.length === 0,
    advanced,
    loopOrResetDetected: resetEvents.length > 0,
    resetEvents,
    stagnationEvents,
    excessivePauseEvents,
    maximumNewPauseSeconds,
    observedUpdateDurationSeconds: Number(
      ((samples.at(-1).observedAtMs - samples[0].observedAtMs) / 1_000).toFixed(9),
    ),
    remainingDurationSeconds: Number(remainingDurationSeconds.toFixed(9)),
    minimumRemainingDurationSeconds: 2,
    remainingWindowPassed,
    reason:
      resetEvents.length > 0
        ? '参数更新阶段 PTS 回退，检测到跨 loop 或时间轴重置'
        : stagnationEvents.length > 0
          ? '参数更新阶段 PTS 停滞，不能证明画面连续推进'
          : excessivePauseEvents.length > 0
            ? '参数更新阶段相对单调时钟的新增停顿超过一个视频帧'
            : !advanced
              ? '参数更新阶段 PTS 未推进，不能证明连续播放'
              : !remainingWindowPassed
                ? '参数更新结束时样本剩余播放窗口不足两秒'
                : undefined,
  };
}

function compileLogLines(text) {
  return String(text).split(/\r?\n/).filter((line) => SHADER_COMPILE.test(line));
}

export function shaderCompilationEvidence(
  beforeUpdates,
  afterUpdates,
  sourceContractMatched = false,
  mpvIdentity = DEFAULT_MPV_IDENTITY,
) {
  const identity = validateMpvIdentity(mpvIdentity);
  const beforeStreams = typeof beforeUpdates === 'string' ? { combined: beforeUpdates } : beforeUpdates;
  const afterStreams = typeof afterUpdates === 'string' ? { combined: afterUpdates } : afterUpdates;
  const streamNames = [...new Set([...Object.keys(beforeStreams), ...Object.keys(afterStreams)])];
  const continuous = streamNames.every((name) =>
    String(afterStreams[name] ?? '').startsWith(String(beforeStreams[name] ?? '')),
  );
  const beforeLog = streamNames.map((name) => String(beforeStreams[name] ?? '')).join('\n');
  const updateLog = streamNames
    .map((name) => {
      const before = String(beforeStreams[name] ?? '');
      const after = String(afterStreams[name] ?? '');
      return after.startsWith(before) ? after.slice(before.length) : after;
    })
    .join('\n');
  const beforeLines = compileLogLines(beforeLog);
  const updateLines = compileLogLines(updateLog);
  return {
    status: updateLines.length > 0 ? 'failed' : sourceContractMatched && continuous ? 'verified' : 'unverified',
    telemetryReliable: sourceContractMatched && continuous,
    logContinuity: continuous,
    compileLinesBeforeUpdates: beforeLines,
    compileLinesDuringUpdates: updateLines,
    repeatedCompileDetected: updateLines.length > 0,
    reason:
      updateLines.length > 0
        ? '参数更新期间日志出现 shader 编译候选'
        : !continuous
          ? '更新前后日志不连续，无法证明参数更新期间没有重复编译'
          : sourceContractMatched
          ? '精确 mpv 提交缓存 user hook、原位更新参数且播放中保留 renderer cache；运行日志未出现重复编译候选'
          : '日志未出现重复编译候选，但 mpv 版本不匹配已审计源码提交',
    sourceContract: {
      matched: sourceContractMatched,
      ref: identity.sourceRef,
      url: identity.sourceUrl,
    },
  };
}

export function parseShaderParameters(source) {
  const lines = String(source).split(/\r?\n/);
  const parameters = [];
  for (let index = 0; index < lines.length; index += 1) {
    const match = lines[index].match(/^\/\/!PARAM\s+([A-Za-z_][A-Za-z0-9_]*)\s*$/);
    if (!match || !match[1].startsWith('al_')) continue;
    let minimum = -1;
    let maximum = 1;
    for (let cursor = index + 1; cursor < Math.min(lines.length, index + 8); cursor += 1) {
      const minMatch = lines[cursor].match(/^\/\/!MINIMUM\s+(-?\d+(?:\.\d+)?)\s*$/);
      const maxMatch = lines[cursor].match(/^\/\/!MAXIMUM\s+(-?\d+(?:\.\d+)?)\s*$/);
      if (minMatch) minimum = Number(minMatch[1]);
      if (maxMatch) maximum = Number(maxMatch[1]);
      if (lines[cursor].startsWith('//!PARAM') || lines[cursor].startsWith('//!HOOK')) break;
    }
    if (!Number.isFinite(minimum) || !Number.isFinite(maximum) || minimum > maximum) {
      throw new Error(`shader 参数 ${match[1]} 范围无效`);
    }
    parameters.push({ name: match[1], minimum, maximum });
  }
  if (!source.includes('//!HOOK MAIN') || parameters.length === 0) {
    throw new Error('gpu83.hook 缺少 MAIN hook 或受控参数');
  }
  if (new Set(parameters.map(({ name }) => name)).size !== parameters.length) {
    throw new Error('gpu83.hook 含重复参数');
  }
  return parameters;
}

export function verifyShaderContract(source, parameters = parseShaderParameters(source)) {
  const sha256 = createHash('sha256').update(source).digest('hex');
  if (sha256 !== VERIFIED_SHADER_SHA256 || parameters.length !== VERIFIED_SHADER_PARAMETER_COUNT) {
    throw new Error(
      `gpu83.hook 与已审计契约不匹配：sha256=${sha256}，参数=${parameters.length}`,
    );
  }
  return { sha256, dynamicParameterCount: parameters.length, status: 'matched' };
}

export function buildShaderOptions(parameters, updateIndex) {
  if (!Number.isSafeInteger(updateIndex) || updateIndex < 0) throw new Error('updateIndex 无效');
  return parameters
    .map(({ name, minimum, maximum }, parameterIndex) => {
      if (!/^al_[A-Za-z0-9_]+$/.test(name)) throw new Error(`不安全的 shader 参数：${name}`);
      const fraction = ((updateIndex + parameterIndex * 3) % 17) / 16;
      const value = minimum + (maximum - minimum) * fraction;
      return `${name}=${Number(value.toFixed(6))}`;
    })
    .join(',');
}

export function validateUpdateOptionSnapshots(updateOptionSnapshots, shaderParameters = null) {
  if (updateOptionSnapshots === null) return null;
  if (
    !Array.isArray(updateOptionSnapshots) || updateOptionSnapshots.length === 0 ||
    updateOptionSnapshots.length > MAX_UPDATE_OPTION_SNAPSHOTS
  ) throw new Error('updateOptionSnapshots 必须是非空且有界的字符串数组');
  if (
    !Array.isArray(shaderParameters) || shaderParameters.length === 0 ||
    shaderParameters.some(({ name }) => typeof name !== 'string' || !/^al_[A-Za-z0-9_]+$/.test(name))
  ) throw new Error('updateOptionSnapshots 需要非空 shaderParameters 证明完整字段集合');
  let totalBytes = 0;
  const expectedNames = shaderParameters.map(({ name }) => name);
  if (new Set(expectedNames).size !== expectedNames.length) throw new Error('shaderParameters 含重复字段');
  const expectedNameSet = new Set(expectedNames);
  const parameterByName = new Map(shaderParameters.map((parameter) => [parameter.name, parameter]));
  let snapshotOrder = null;
  const snapshots = updateOptionSnapshots.map((snapshot) => {
    if (typeof snapshot !== 'string') throw new Error('updateOptionSnapshots 只能包含字符串');
    const bytes = Buffer.byteLength(snapshot, 'utf8');
    totalBytes += bytes;
    if (bytes === 0 || bytes > MAX_UPDATE_OPTION_SNAPSHOT_BYTES || !/^[\x20-\x7e]+$/.test(snapshot)) {
      throw new Error('updateOptionSnapshots 含空值、超长值或非 ASCII 可打印字符');
    }
    const entries = snapshot.split(',');
    if (entries.length === 0 || entries.length > 256) throw new Error('updateOptionSnapshots 参数数量无效');
    const names = [];
    const seen = new Set();
    for (const entry of entries) {
      const match = /^(al_[A-Za-z0-9_]+)=[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.exec(entry);
      if (!match || seen.has(match[1])) throw new Error('updateOptionSnapshots 含非法或重复参数');
      const value = Number(entry.slice(entry.indexOf('=') + 1));
      const parameter = parameterByName.get(match[1]);
      if (!Number.isFinite(value)) throw new Error('updateOptionSnapshots 含非有限参数值');
      if (Number.isFinite(parameter?.minimum) && value < parameter.minimum
        || Number.isFinite(parameter?.maximum) && value > parameter.maximum) {
        throw new Error(`updateOptionSnapshots 参数越界：${match[1]}=${value}`);
      }
      seen.add(match[1]);
      names.push(match[1]);
    }
    if (names.length !== expectedNames.length || names.some((name) => !expectedNameSet.has(name))) {
      throw new Error('updateOptionSnapshots 必须使用完整参数集合，且精确等于 shaderParameters 字段');
    }
    if (snapshotOrder === null) snapshotOrder = names;
    else if (names.some((name, index) => name !== snapshotOrder[index])) {
      throw new Error('updateOptionSnapshots 各快照必须使用相同字段顺序');
    }
    return snapshot;
  });
  if (totalBytes > MAX_UPDATE_OPTION_SNAPSHOTS_BYTES) throw new Error('updateOptionSnapshots 总大小超过上限');
  return snapshots;
}

const CPU4_FIELDS = Object.freeze({
  brightness_percent: Object.freeze({ minimum: -100, maximum: 100, target: 'eq@autolive_cpu4_eq', command: 'brightness', convert: (value) => value / 100 }),
  contrast_percent: Object.freeze({ minimum: 0, maximum: 200, target: 'eq@autolive_cpu4_eq', command: 'contrast', convert: (value) => value / 100 }),
  saturation_percent: Object.freeze({ minimum: 0, maximum: 200, target: 'eq@autolive_cpu4_eq', command: 'saturation', convert: (value) => value / 100 }),
  hue_rotation_degrees: Object.freeze({ minimum: -180, maximum: 180, target: 'hue@autolive_cpu4_hue', command: 'h', convert: (value) => value }),
});

export function compileCpu4Update(input) {
  if (input === null || typeof input !== 'object' || Array.isArray(input)) throw new Error('CPU4 更新必须是对象');
  const keys = Object.keys(input).sort();
  const expected = Object.keys(CPU4_FIELDS).sort();
  if (keys.length !== expected.length || keys.some((key, index) => key !== expected[index])) {
    throw new Error('CPU4 只接受 brightness/contrast/saturation/hue 四字段完整快照');
  }
  return Object.keys(CPU4_FIELDS).map((field) => {
    const rule = CPU4_FIELDS[field];
    const value = finiteNumber(input[field], field, rule.minimum, rule.maximum);
    return ['vf-command', 'autolive_cpu4', rule.command, String(rule.convert(value)), rule.target];
  });
}

export async function sendCpu4CommandWithRetry(
  ipc,
  command,
  maxAttempts = 8,
  wait = sleep,
) {
  if (!Number.isSafeInteger(maxAttempts) || maxAttempts <= 0) throw new Error('CPU4 命令重试上限无效');
  let lastError;
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    try {
      await ipc.send(command);
      return attempt - 1;
    } catch (error) {
      lastError = error;
      if (attempt < maxAttempts) await wait(25);
    }
  }
  throw new Error(
    `CPU4 命令 ${JSON.stringify(command)} 连续 ${maxAttempts} 次失败：${lastError instanceof Error ? lastError.message : String(lastError)}`,
  );
}

export function cpu4Snapshot(index) {
  const phase = index % 2 === 0 ? 1 : -1;
  return {
    brightness_percent: phase * 8,
    contrast_percent: 100 + phase * 7,
    saturation_percent: 100 + phase * 9,
    hue_rotation_degrees: phase * 12,
  };
}

export function buildMpvArguments({
  backend, mediaPath, shaderPath, shaderPaths = null, pipeName, renderSize, cpu4 = false,
  hiddenWindow = false, fullscreenWindow = false, allowNoShaderStart = false,
}) {
  assertManagedPipe(pipeName);
  if (!['d3d11', 'vulkan', 'cpu4'].includes(backend)) throw new Error(`不支持的 mpv 后端：${backend}`);
  if (cpu4 !== (backend === 'cpu4')) throw new Error('CPU4 启动标记与后端不一致');
  const width = finiteNumber(renderSize?.width, 'render width', 1, 8192);
  const height = finiteNumber(renderSize?.height, 'render height', 1, 8192);
  const argumentsList = [
    '--no-config',
    '--input-default-bindings=no',
    '--input-vo-keyboard=no',
    '--terminal=yes',
    '--input-terminal=no',
    '--idle=no',
    '--loop-file=inf',
    '--force-window=immediate',
    '--keep-open=yes',
    '--audio=no',
    '--pause=no',
    '--vo=gpu-next',
    `--geometry=${width}x${height}`,
    '--msg-level=all=warn,vo/gpu-next=info,libplacebo=info',
    `--input-ipc-server=${pipeName}`,
  ];
  if (hiddenWindow) {
    argumentsList.push(
      '--focus-on=never',
      '--show-in-taskbar=no',
      '--border=no',
      '--video-unscaled=yes',
      '--hidpi-window-scale=no',
      '--auto-window-resize=no',
    );
  }
  if (fullscreenWindow) {
    argumentsList.push('--fullscreen=yes', '--fs-screen=current');
  }
  if (backend === 'd3d11') argumentsList.push('--gpu-api=d3d11', '--gpu-context=d3d11', '--hwdec=d3d11va');
  if (backend === 'vulkan') argumentsList.push('--gpu-api=vulkan', '--gpu-context=winvk', '--hwdec=d3d11va-copy');
  if (backend === 'cpu4') argumentsList.push('--gpu-api=d3d11', '--gpu-context=d3d11', '--hwdec=no', `--vf=${CPU4_FILTER}`);
  if (!cpu4) {
    if (shaderPaths === null) {
      argumentsList.push(`--glsl-shaders=${shaderPath}`);
    } else {
      if (!Array.isArray(shaderPaths) || shaderPaths.length > 16
        || shaderPaths.some((path) => typeof path !== 'string' || path.length === 0)) {
        throw new Error('GPU shader 文件列表无效');
      }
      if (shaderPaths.length === 0 && allowNoShaderStart !== true) {
        throw new Error('零 shader 启动必须显式启用 allowNoShaderStart');
      }
      argumentsList.push(...shaderPaths.map((path) => `--glsl-shader=${path}`));
    }
  }
  argumentsList.push('--', mediaPath);
  return argumentsList;
}

async function assertRegularFile(path, label) {
  await access(path);
  if (!(await stat(path)).isFile()) throw new Error(`${label} 不是普通文件：${path}`);
}

async function sha256File(path) {
  return createHash('sha256').update(await readFile(path)).digest('hex');
}

async function runCaptured(executable, argumentsList, { timeoutMs = 30_000, maxBytes = MAX_PROCESS_OUTPUT_BYTES } = {}) {
  return await new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(executable, argumentsList, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    const chunks = [];
    let bytes = 0;
    let overflow = false;
    const append = (chunk) => {
      bytes += chunk.length;
      if (bytes <= maxBytes) chunks.push(chunk);
      else overflow = true;
    };
    child.stdout.on('data', append);
    child.stderr.on('data', append);
    const timer = setTimeout(() => {
      child.kill();
      rejectPromise(new Error(`${executable} 执行超时`));
    }, timeoutMs);
    child.once('error', (error) => {
      clearTimeout(timer);
      rejectPromise(error);
    });
    child.once('exit', (code, signal) => {
      clearTimeout(timer);
      const output = Buffer.concat(chunks).toString('utf8');
      if (overflow) return rejectPromise(new Error(`${executable} 输出超过大小上限`));
      if (code !== 0) return rejectPromise(new Error(`${executable} 退出码 ${code ?? signal}: ${output.slice(-4000)}`));
      resolvePromise(output);
    });
  });
}

export async function generateSamples(
  ffmpegPath,
  directory,
  run = runCaptured,
  sampleSpecs = SAMPLE_SPECS,
) {
  if (!Array.isArray(sampleSpecs) || sampleSpecs.length === 0 || sampleSpecs.length > 16) {
    throw new Error('验收样本规格必须是非空且有界的数组');
  }
  const samples = [];
  for (const spec of sampleSpecs) {
    if (
      !/^[A-Za-z0-9-]+$/.test(spec?.name) ||
      !Number.isSafeInteger(spec.width) || spec.width < 1 || spec.width > 8192 ||
      !Number.isSafeInteger(spec.height) || spec.height < 1 || spec.height > 8192 ||
      !Number.isSafeInteger(spec.fps) || spec.fps < 1 || spec.fps > 240 ||
      typeof spec.duration !== 'number' || !Number.isFinite(spec.duration) ||
      spec.duration < 0.25 || spec.duration > 60 ||
      typeof spec.gateRole !== 'string' || spec.gateRole.length === 0 || spec.gateRole.length > 64
    ) throw new Error('验收样本规格无效');
    const path = join(directory, `${spec.name}.mp4`);
    const argumentsList = [
      '-hide_banner', '-loglevel', 'error', '-y',
      '-f', 'lavfi', '-i', `testsrc2=size=${spec.width}x${spec.height}:rate=${spec.fps}:duration=${spec.duration}`,
      '-an', '-c:v', 'libopenh264', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', path,
    ];
    await run(ffmpegPath, argumentsList, { timeoutMs: 60_000 });
    await assertRegularFile(path, spec.name);
    samples.push({ ...spec, path, bytes: (await stat(path)).size });
  }
  return samples;
}

async function connectPipe(pipeName, timeoutMs = 5_000) {
  assertManagedPipe(pipeName);
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      return await new Promise((resolvePromise, rejectPromise) => {
        const socket = createConnection(pipeName);
        socket.once('connect', () => resolvePromise(socket));
        socket.once('error', (error) => {
          socket.destroy();
          rejectPromise(error);
        });
      });
    } catch (error) {
      lastError = error;
      await sleep(25);
    }
  }
  throw new Error(`连接 mpv 命名管道超时：${lastError?.message ?? 'unknown'}`);
}

export class JsonIpcClient {
  constructor(socket, { maxResponseBytes = MAX_IPC_RESPONSE_BYTES } = {}) {
    if (!Number.isSafeInteger(maxResponseBytes) || maxResponseBytes <= 0) throw new Error('响应大小上限无效');
    this.socket = socket;
    this.maxResponseBytes = maxResponseBytes;
    this.buffer = Buffer.alloc(0);
    this.pending = new Map();
    this.retiredRequestIds = new Set();
    this.nextRequestId = 1;
    this.closedError = null;
    socket.on('data', (chunk) => this.#receive(chunk));
    socket.on('error', (error) => this.#fail(error));
    socket.on('close', () => this.#fail(new Error('mpv IPC 已关闭')));
  }

  #receive(chunk) {
    if (this.closedError) return;
    this.buffer = Buffer.concat([this.buffer, chunk]);
    if (this.buffer.length > this.maxResponseBytes && this.buffer.indexOf(0x0a) === -1) {
      this.#fail(new Error('mpv IPC 单行响应超过大小上限'));
      return;
    }
    for (let newline = this.buffer.indexOf(0x0a); newline !== -1; newline = this.buffer.indexOf(0x0a)) {
      const line = this.buffer.subarray(0, newline);
      this.buffer = this.buffer.subarray(newline + 1);
      let response;
      try {
        response = parseIpcResponseLine(line, this.maxResponseBytes);
      } catch (error) {
        this.#fail(error);
        return;
      }
      if (response.request_id === undefined) continue;
      if (!Number.isSafeInteger(response.request_id)) {
        this.#fail(new Error(`未知或重复 request_id：${response.request_id}`));
        return;
      }
      if (this.retiredRequestIds.delete(response.request_id)) continue;
      if (!this.pending.has(response.request_id)) {
        this.#fail(new Error(`未知或重复 request_id：${response.request_id}`));
        return;
      }
      const pending = this.pending.get(response.request_id);
      this.pending.delete(response.request_id);
      clearTimeout(pending.timer);
      if (response.error !== 'success') pending.reject(new Error(`mpv 请求失败：${response.error ?? 'missing error'}`));
      else pending.resolve(response);
    }
    if (this.buffer.length > this.maxResponseBytes) this.#fail(new Error('mpv IPC 单行响应超过大小上限'));
  }

  #fail(error) {
    if (this.closedError) return;
    this.closedError = error instanceof Error ? error : new Error(String(error));
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(this.closedError);
    }
    this.pending.clear();
    this.retiredRequestIds.clear();
    this.socket.destroy();
  }

  #retireRequest(requestId) {
    this.retiredRequestIds.add(requestId);
    while (this.retiredRequestIds.size > MAX_RETIRED_REQUEST_IDS) {
      this.retiredRequestIds.delete(this.retiredRequestIds.values().next().value);
    }
  }

  async send(command, deadlineMs = IPC_DEADLINE_MS) {
    if (this.closedError) throw this.closedError;
    if (!Number.isSafeInteger(deadlineMs) || deadlineMs <= 0) throw new Error('IPC deadline 无效');
    const requestId = this.nextRequestId;
    this.nextRequestId += 1;
    const line = encodeIpcRequest(command, requestId);
    return await new Promise((resolvePromise, rejectPromise) => {
      const timer = setTimeout(() => {
        this.pending.delete(requestId);
        this.#retireRequest(requestId);
        rejectPromise(new Error(`mpv IPC 请求 ${requestId} 超时`));
      }, deadlineMs);
      this.pending.set(requestId, { resolve: resolvePromise, reject: rejectPromise, timer });
      this.socket.write(line, (error) => {
        if (!error) return;
        clearTimeout(timer);
        this.pending.delete(requestId);
        rejectPromise(error);
      });
    });
  }

  close() {
    this.#fail(new Error('mpv IPC 客户端关闭'));
  }
}

function boundedOutputCollector(stream, maxBytes = MAX_PROCESS_OUTPUT_BYTES) {
  const chunks = [];
  let bytes = 0;
  let overflow = false;
  stream.on('data', (chunk) => {
    bytes += chunk.length;
    if (bytes <= maxBytes) chunks.push(chunk);
    else overflow = true;
  });
  return () => ({ text: Buffer.concat(chunks).toString('utf8'), bytes, overflow });
}

async function waitForExit(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) return true;
  return await new Promise((resolvePromise) => {
    const timer = setTimeout(() => {
      child.off('exit', onExit);
      resolvePromise(false);
    }, timeoutMs);
    const onExit = () => {
      clearTimeout(timer);
      resolvePromise(true);
    };
    child.once('exit', onExit);
  });
}

async function terminateMpv(child, ipc) {
  try {
    if (ipc) await ipc.send(['quit'], 500);
  } catch {}
  ipc?.close();
  if (await waitForExit(child, 1_000)) return;
  child.kill();
  if (await waitForExit(child, 1_000)) return;
  if (process.platform === 'win32' && child.pid) {
    try {
      await runCaptured('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { timeoutMs: 2_000, maxBytes: 8 * 1024 });
    } catch {}
  }
  await waitForExit(child, 1_000);
}

async function waitForVo(ipc) {
  const deadline = Date.now() + STARTUP_DEADLINE_MS;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const response = await ipc.send(['get_property', 'vo-configured'], 750);
      if (response.data === true) return true;
    } catch (error) {
      lastError = error;
    }
    await sleep(50);
  }
  throw new Error(`vo-configured 未在期限内变为 true${lastError ? `：${lastError.message}` : ''}`);
}

async function waitForFirstFrame(ipc) {
  const deadline = Date.now() + STARTUP_DEADLINE_MS;
  let initialPosition = null;
  while (Date.now() < deadline) {
    const position = await optionalProperty(ipc, 'time-pos');
    if (typeof position === 'number' && Number.isFinite(position)) {
      if (initialPosition === null) initialPosition = position;
      else if (position !== initialPosition) return position;
    }
    await sleep(25);
  }
  throw new Error('mpv 播放位置未在期限内推进，不能证明首帧已进入活动滤镜图');
}

async function optionalProperty(ipc, property) {
  try {
    return (await ipc.send(['get_property', property])).data ?? null;
  } catch {
    return null;
  }
}

async function dropFrameCounters(ipc) {
  return {
    frameDropCount: await optionalProperty(ipc, 'frame-drop-count'),
    decoderFrameDropCount: await optionalProperty(ipc, 'decoder-frame-drop-count'),
  };
}

async function readUpdatePlaybackPosition(ipc) {
  const position = await optionalProperty(ipc, 'time-pos/full');
  if (typeof position !== 'number' || !Number.isFinite(position) || position < 0) {
    throw new Error('参数更新阶段缺少有限的 time-pos/full PTS');
  }
  return position;
}

async function captureUpdatePlaybackPosition(ipc) {
  const mediaPtsSeconds = await readUpdatePlaybackPosition(ipc);
  return { mediaPtsSeconds, observedAtMs: performance.now() };
}

function shaderFailures(stderr) {
  return stderr.split(/\r?\n/).filter((line) => SHADER_ERROR.test(line)).slice(0, 20);
}

export function classifyAttemptError(error) {
  const message = error instanceof Error ? error.message : String(error);
  if (/帧预算未通过|分辨率证据不符|参数更新期间重启或退出|PID 在更新期间发生变化|stdout\/stderr 超过|检测到 shader\/hook 错误/i.test(message)) {
    return { status: 'failed', error: message };
  }
  const unavailable = /ENOENT|not found|无法找到|vo-configured|vulkan|d3d11|d3d11va|winvk|连接 mpv 命名管道/i.test(message);
  return { status: unavailable ? 'unavailable' : 'failed', error: message };
}

export function requireStableSessionPid(startPid, ipcPid, childPid, stage) {
  if (
    !Number.isSafeInteger(startPid) || startPid <= 0 ||
    !Number.isSafeInteger(ipcPid) || ipcPid <= 0 ||
    !Number.isSafeInteger(childPid) || childPid <= 0 ||
    startPid !== ipcPid || startPid !== childPid
  ) {
    throw new Error(`mpv PID 在${stage}不完整或不一致：${startPid ?? 'none'}/${ipcPid ?? 'none'}/${childPid ?? 'none'}`);
  }
  return ipcPid;
}

export function renderTargetMatchesSample(sample, osdDimensions) {
  return Number.isSafeInteger(sample?.width) && sample.width > 0 &&
    Number.isSafeInteger(sample?.height) && sample.height > 0 &&
    osdDimensions?.w === sample.width && osdDimensions?.h === sample.height;
}

export function qualifyFrameBudgetForRenderTarget(frameBudget, renderTargetMatched) {
  if (renderTargetMatched || frameBudget.status !== 'passed') return frameBudget;
  return {
    ...frameBudget,
    status: 'unverified',
    reason: '实际渲染表面与样本分辨率不一致，原始计时样本仅供诊断，不构成全分辨率帧预算通过证据',
  };
}

export async function runMpvAttempt({
  mpvPath, shaderPath, shaderPaths = null, sample, backend, shaderParameters, sourceContractMatched,
  mpvIdentity = DEFAULT_MPV_IDENTITY,
  updateCount = GPU_UPDATE_COUNT, hiddenWindow = false, fullscreenWindow = false,
  allowNoShaderStart = false,
  sessionSetup = null, sessionProbe = null, updateOptionSnapshots = null,
  requireUpdatePlaybackContinuity = false,
}) {
  if (sessionSetup !== null && typeof sessionSetup !== 'function') throw new Error('sessionSetup 必须是函数');
  if (sessionProbe !== null && typeof sessionProbe !== 'function') throw new Error('sessionProbe 必须是函数');
  if (typeof requireUpdatePlaybackContinuity !== 'boolean') throw new Error('requireUpdatePlaybackContinuity 必须是布尔值');
  const fixedOptionSnapshots = validateUpdateOptionSnapshots(updateOptionSnapshots, shaderParameters);
  const pipeName = managedPipeName();
  const renderSize = { width: sample.width, height: sample.height };
  const argumentsList = buildMpvArguments({
    backend, mediaPath: sample.path, shaderPath, shaderPaths, pipeName, renderSize, cpu4: false,
    hiddenWindow, fullscreenWindow, allowNoShaderStart,
  });
  const startedAt = new Date().toISOString();
  const child = spawn(mpvPath, argumentsList, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  const readStdout = boundedOutputCollector(child.stdout);
  const readStderr = boundedOutputCollector(child.stderr);
  let ipc;
  const pid = child.pid ?? null;
  try {
    ipc = new JsonIpcClient(await connectPipe(pipeName));
    await waitForVo(ipc);
    await waitForFirstFrame(ipc);
    const ipcPidBeforeSetup = requireStableSessionPid(
      pid, await optionalProperty(ipc, 'pid'), child.pid ?? null, 'sessionSetup 前',
    );
    const sessionSetupEvidence = sessionSetup === null
      ? null
      : await sessionSetup({ ipc, child, pid, sample, backend });
    const ipcPidAfterSetup = requireStableSessionPid(
      pid, await optionalProperty(ipc, 'pid'), child.pid ?? null, 'sessionSetup 后',
    );
    const logsBeforeUpdates = { stdout: readStdout().text, stderr: readStderr().text };
    const ipcPidBeforeProbe = requireStableSessionPid(
      pid, await optionalProperty(ipc, 'pid'), child.pid ?? null, 'sessionProbe 前',
    );
    const dropsBeforeProbe = await dropFrameCounters(ipc);
    const sessionProbeEvidence = sessionProbe === null
      ? null
      : await sessionProbe({ ipc, child, pid, sample, backend });
    const ipcPidAfterProbe = requireStableSessionPid(
      pid, await optionalProperty(ipc, 'pid'), child.pid ?? null, 'sessionProbe 后',
    );
    const dropsAfterProbe = await dropFrameCounters(ipc);
    const voPassesBeforeUpdates = await optionalProperty(ipc, 'vo-passes');
    const dropFrameTimeline = [dropsBeforeProbe, dropsAfterProbe];
    const expectedDecoder = backend === 'd3d11' ? 'd3d11va' : 'd3d11va-copy';
    const latencies = [];
    const observedFrameTotalsNs = [];
    let lastTelemetryIdentity = null;
    const observationSettleMs = frameObservationSettleMs(sample);
    const updatePlaybackPositions = requireUpdatePlaybackContinuity
      ? [await captureUpdatePlaybackPosition(ipc)]
      : null;
    for (let index = 0; index < updateCount; index += 1) {
      const options = fixedOptionSnapshots === null
        ? buildShaderOptions(shaderParameters, index)
        : fixedOptionSnapshots[index % fixedOptionSnapshots.length];
      const start = performance.now();
      await ipc.send(['set_property', 'glsl-shader-opts', options]);
      latencies.push(performance.now() - start);
      if (child.exitCode !== null || child.signalCode !== null || child.pid !== pid) throw new Error('mpv 在参数更新期间重启或退出');
      await sleep(observationSettleMs);
      const observation = observeFrameBudgetSample(await optionalProperty(ipc, 'vo-passes'));
      const advancement = advanceFrameBudgetObservation(lastTelemetryIdentity, observation);
      lastTelemetryIdentity = advancement.telemetryIdentity;
      if (advancement.accepted) observedFrameTotalsNs.push(observation.totalNs);
      dropFrameTimeline.push(await dropFrameCounters(ipc));
      if (updatePlaybackPositions !== null) {
        updatePlaybackPositions.push(await captureUpdatePlaybackPosition(ipc));
      }
    }
    await sleep(100);
    const decoder = await optionalProperty(ipc, 'hwdec-current');
    const ipcPidAfter = requireStableSessionPid(
      pid, await optionalProperty(ipc, 'pid'), child.pid ?? null, '参数更新后',
    );
    const sourceVideoParams = await optionalProperty(ipc, 'video-params');
    const outputVideoParams = await optionalProperty(ipc, 'video-out-params');
    const osdDimensions = await optionalProperty(ipc, 'osd-dimensions');
    const voPasses = await optionalProperty(ipc, 'vo-passes');
    const dropsAfterUpdates = await dropFrameCounters(ipc);
    dropFrameTimeline.push(dropsAfterUpdates);
    const voConfigured = (await ipc.send(['get_property', 'vo-configured'])).data === true;
    const evidence = {
      voConfigured,
      decoder,
      currentVo: await optionalProperty(ipc, 'current-vo'),
      containerFps: await optionalProperty(ipc, 'container-fps'),
      estimatedVfFps: await optionalProperty(ipc, 'estimated-vf-fps'),
      frameDropCount: dropsAfterUpdates.frameDropCount,
      decoderFrameDropCount: dropsAfterUpdates.decoderFrameDropCount,
    };
    const stdout = readStdout();
    const stderr = readStderr();
    const combinedLogs = `${stdout.text}\n${stderr.text}`;
    const failures = shaderFailures(combinedLogs);
    const stderrFailures = shaderFailures(stderr.text);
    const compilation = shaderCompilationEvidence(
      logsBeforeUpdates,
      { stdout: stdout.text, stderr: stderr.text },
      sourceContractMatched,
      mpvIdentity,
    );
    const frameBudget = buildFrameBudgetEvidence(
      sample, voPasses, voPassesBeforeUpdates, MIN_FRAME_BUDGET_SAMPLES, observedFrameTotalsNs,
    );
    const dropFrames = buildDropFrameTimelineEvidence(dropFrameTimeline);
    const updatePlaybackTimeline = updatePlaybackPositions === null
      ? null
      : buildUpdatePlaybackTimelineEvidence(updatePlaybackPositions, updateCount, sample);
    const pidUnchanged = [ipcPidBeforeSetup, ipcPidAfterSetup, ipcPidBeforeProbe, ipcPidAfterProbe, ipcPidAfter]
      .every((value) => value === pid) && child.pid === pid;
    const renderTargetMatched = renderTargetMatchesSample(sample, osdDimensions);
    const frameBudgetEvidence = qualifyFrameBudgetForRenderTarget(frameBudget, renderTargetMatched);
    const validationErrors = [];
    if (!voConfigured) validationErrors.push('更新后 vo-configured 不为 true');
    if (stdout.overflow || stderr.overflow) validationErrors.push('mpv stdout/stderr 超过有界捕获上限');
    if (failures.length > 0) validationErrors.push(`检测到 shader/hook 错误：${failures.join(' | ')}`);
    if (!pidUnchanged) {
      validationErrors.push(
        `mpv PID 证据链不一致：start=${pid}，beforeSetup=${ipcPidBeforeSetup}，afterSetup=${ipcPidAfterSetup}，beforeProbe=${ipcPidBeforeProbe}，afterProbe=${ipcPidAfterProbe}，afterUpdates=${ipcPidAfter}，child=${child.pid ?? 'none'}`,
      );
    }
    if (sourceVideoParams?.w !== sample.width || sourceVideoParams?.h !== sample.height) {
      validationErrors.push(`源分辨率证据不符：期望 ${sample.width}x${sample.height}，实际 ${sourceVideoParams?.w ?? 'unknown'}x${sourceVideoParams?.h ?? 'unknown'}`);
    }
    if (!renderTargetMatched) {
      validationErrors.push(`渲染表面分辨率证据不符：期望 ${sample.width}x${sample.height}，实际 ${osdDimensions?.w ?? 'unknown'}x${osdDimensions?.h ?? 'unknown'}`);
    }
    if (frameBudgetEvidence.status === 'failed') validationErrors.push(`GPU 帧预算未通过：P99 ${frameBudgetEvidence.timing?.p99Ms}ms`);
    if (dropFrames.status === 'failed') validationErrors.push('参数更新期间出现视频输出或解码器丢帧');
    if (updatePlaybackTimeline?.status !== undefined && updatePlaybackTimeline.status !== 'passed') {
      validationErrors.push(updatePlaybackTimeline.reason ?? '参数更新阶段 PTS 连续性未通过');
    }
    if (decoder !== expectedDecoder) validationErrors.push(`${backend} 未使用要求的 ${expectedDecoder}，实际为 ${decoder ?? 'none'}`);
    const unverified = frameBudgetEvidence.status === 'unverified' || dropFrames.status === 'unverified' || compilation.status === 'unverified';
    return {
      status: validationErrors.length > 0 || compilation.status === 'failed' ? 'failed' : unverified ? 'unverified' : 'passed',
      error: validationErrors.length > 0 ? validationErrors.join('；') : undefined,
      validationErrors,
      backend, sample: sample.name, sampleGateRole: sample.gateRole, startedAt,
      pidBefore: pid, pidAfter: child.pid ?? null,
      ipcPidBeforeSetup, ipcPidAfterSetup, ipcPidBeforeProbe, ipcPidAfterProbe, ipcPidAfter, pidUnchanged,
      updateLatency: summarizeLatencies(latencies), evidence,
      resolutionEvidence: {
        source: {
          expectedWidth: sample.width,
          expectedHeight: sample.height,
          actualWidth: sourceVideoParams?.w ?? null,
          actualHeight: sourceVideoParams?.h ?? null,
        },
        decodedOutput: outputVideoParams,
        windowRenderTarget: {
          requested: renderSize,
          actual: osdDimensions,
          matched: renderTargetMatched,
        },
        fullResolutionRenderBudget: renderTargetMatched ? 'measured' : 'unverified',
        note:
          renderTargetMatched
            ? '源帧与窗口渲染目标均为样本分辨率'
            : '实际窗口渲染目标与样本分辨率不一致，本次帧预算不得通过',
      },
      frameBudgetEvidence,
      dropFrameEvidence: dropFrames,
      updatePlaybackTimelineEvidence: updatePlaybackTimeline,
      shaderCompilationEvidence: compilation,
      sessionSetupEvidence,
      sessionProbeEvidence,
      updateOptionSnapshotCount: fixedOptionSnapshots?.length ?? null,
      shaderParameterCount: shaderParameters.length, stdoutBytes: stdout.bytes, stderrBytes: stderr.bytes,
      stderrShaderErrors: stderrFailures, shaderErrors: failures,
    };
  } catch (error) {
    const stdout = readStdout();
    const stderr = readStderr();
    return {
      ...classifyAttemptError(error), backend, sample: sample.name, startedAt, pidBefore: pid, pidAfter: child.pid ?? null,
      pidUnchanged: false,
      stdoutBytes: stdout.bytes, stdoutOverflow: stdout.overflow,
      stdoutTail: stdout.text.split(/\r?\n/).filter(Boolean).slice(-20),
      stderrBytes: stderr.bytes, stderrOverflow: stderr.overflow,
      stderrTail: stderr.text.split(/\r?\n/).filter(Boolean).slice(-20),
      shaderErrors: shaderFailures(`${stdout.text}\n${stderr.text}`),
    };
  } finally {
    await terminateMpv(child, ipc);
  }
}

async function runCpu4Attempt({
  mpvPath, sample, updateCount = CPU4_UPDATE_COUNT, hiddenWindow = false, fullscreenWindow = false,
}) {
  const pipeName = managedPipeName();
  const renderSize = { width: sample.width, height: sample.height };
  const argumentsList = buildMpvArguments({
    backend: 'cpu4', mediaPath: sample.path, shaderPath: '', pipeName, renderSize, cpu4: true,
    hiddenWindow, fullscreenWindow,
  });
  const child = spawn(mpvPath, argumentsList, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  const readStdout = boundedOutputCollector(child.stdout);
  const readStderr = boundedOutputCollector(child.stderr);
  let ipc;
  const pid = child.pid ?? null;
  try {
    ipc = new JsonIpcClient(await connectPipe(pipeName));
    await waitForVo(ipc);
    const firstFrameTimePos = await waitForFirstFrame(ipc);
    const voPassesBeforeUpdates = await optionalProperty(ipc, 'vo-passes');
    const dropsBeforeUpdates = await dropFrameCounters(ipc);
    const dropFrameTimeline = [dropsBeforeUpdates];
    const ipcPidBefore = await optionalProperty(ipc, 'pid');
    const latencies = [];
    const observedFrameTotalsNs = [];
    let lastTelemetryIdentity = null;
    const observationSettleMs = frameObservationSettleMs(sample);
    let transientCommandRetries = 0;
    for (let index = 0; index < updateCount; index += 1) {
      const commands = compileCpu4Update(cpu4Snapshot(index));
      const start = performance.now();
      for (const command of commands) {
        transientCommandRetries += await sendCpu4CommandWithRetry(ipc, command);
      }
      latencies.push(performance.now() - start);
      if (child.exitCode !== null || child.signalCode !== null || child.pid !== pid) throw new Error('mpv 在 CPU4 更新期间重启或退出');
      await sleep(observationSettleMs);
      const observation = observeFrameBudgetSample(await optionalProperty(ipc, 'vo-passes'));
      const advancement = advanceFrameBudgetObservation(lastTelemetryIdentity, observation);
      lastTelemetryIdentity = advancement.telemetryIdentity;
      if (advancement.accepted) observedFrameTotalsNs.push(observation.totalNs);
      dropFrameTimeline.push(await dropFrameCounters(ipc));
    }
    await sleep(100);
    const voPasses = await optionalProperty(ipc, 'vo-passes');
    const dropsAfterUpdates = await dropFrameCounters(ipc);
    dropFrameTimeline.push(dropsAfterUpdates);
    const ipcPidAfter = await optionalProperty(ipc, 'pid');
    const voConfigured = (await ipc.send(['get_property', 'vo-configured'])).data === true;
    const currentVo = await optionalProperty(ipc, 'current-vo');
    const sourceVideoParams = await optionalProperty(ipc, 'video-params');
    const outputVideoParams = await optionalProperty(ipc, 'video-out-params');
    const osdDimensions = await optionalProperty(ipc, 'osd-dimensions');
    const stdout = readStdout();
    const stderr = readStderr();
    const filterErrors = `${stdout.text}\n${stderr.text}`.split(/\r?\n/).filter((line) => CPU4_ERROR.test(line)).slice(0, 20);
    const rawFrameBudget = buildFrameBudgetEvidence(
      sample, voPasses, voPassesBeforeUpdates, MIN_FRAME_BUDGET_SAMPLES, observedFrameTotalsNs,
    );
    const renderTargetMatched = renderTargetMatchesSample(sample, osdDimensions);
    const frameBudget = qualifyFrameBudgetForRenderTarget(rawFrameBudget, renderTargetMatched);
    const dropFrames = buildDropFrameTimelineEvidence(dropFrameTimeline);
    const pidUnchanged =
      pid !== null && child.pid === pid &&
      (ipcPidBefore === null || ipcPidBefore === pid) &&
      (ipcPidAfter === null || ipcPidAfter === pid);
    const validationErrors = [];
    if (!voConfigured) validationErrors.push('CPU4 更新后 vo-configured 不为 true');
    if (!pidUnchanged) validationErrors.push(`CPU4 mpv PID 在更新期间发生变化：${pid}/${ipcPidBefore} -> ${child.pid}/${ipcPidAfter}`);
    if (stdout.overflow || stderr.overflow) validationErrors.push('CPU4 mpv stdout/stderr 超过有界捕获上限');
    if (filterErrors.length > 0) validationErrors.push(`检测到 CPU4 滤镜错误：${filterErrors.join(' | ')}`);
    if (sourceVideoParams?.w !== sample.width || sourceVideoParams?.h !== sample.height) {
      validationErrors.push(`CPU4 源分辨率证据不符：期望 ${sample.width}x${sample.height}，实际 ${sourceVideoParams?.w ?? 'unknown'}x${sourceVideoParams?.h ?? 'unknown'}`);
    }
    if (!renderTargetMatched) {
      validationErrors.push(`CPU4 渲染表面分辨率证据不符：期望 ${sample.width}x${sample.height}，实际 ${osdDimensions?.w ?? 'unknown'}x${osdDimensions?.h ?? 'unknown'}`);
    }
    if (frameBudget.status === 'failed') validationErrors.push(`CPU4 帧预算未通过：P99 ${frameBudget.timing?.p99Ms}ms`);
    if (dropFrames.status === 'failed') validationErrors.push('CPU4 参数更新期间出现视频输出或解码器丢帧');
    const unverified = frameBudget.status === 'unverified' || dropFrames.status === 'unverified';
    return {
      status: validationErrors.length > 0 ? 'failed' : unverified ? 'unverified' : 'passed',
      error: validationErrors.length > 0 ? validationErrors.join('；') : undefined,
      validationErrors,
      backend: 'cpu4', sample: sample.name, pidBefore: pid, pidAfter: child.pid ?? null,
      ipcPidBefore, ipcPidAfter, pidUnchanged, updateLatency: summarizeLatencies(latencies),
      updateCount, commandsPerUpdate: 4, filter: CPU4_FILTER, voConfigured, currentVo, firstFrameTimePos,
      transientCommandRetries,
      filterErrors, stdoutBytes: stdout.bytes, stderrBytes: stderr.bytes,
      resolutionEvidence: {
        source: {
          expectedWidth: sample.width,
          expectedHeight: sample.height,
          actualWidth: sourceVideoParams?.w ?? null,
          actualHeight: sourceVideoParams?.h ?? null,
        },
        decodedOutput: outputVideoParams,
        windowRenderTarget: { requested: renderSize, actual: osdDimensions, matched: renderTargetMatched },
        fullResolutionRenderBudget: renderTargetMatched ? 'measured' : 'unverified',
      },
      frameBudgetEvidence: frameBudget,
      dropFrameEvidence: dropFrames,
    };
  } catch (error) {
    const stdout = readStdout();
    const stderr = readStderr();
    return {
      ...classifyAttemptError(error), backend: 'cpu4', sample: sample.name, pidBefore: pid, pidAfter: child.pid ?? null,
      pidUnchanged: pid !== null && child.pid === pid,
      stdoutBytes: stdout.bytes, stdoutOverflow: stdout.overflow,
      stdoutTail: stdout.text.split(/\r?\n/).filter(Boolean).slice(-20),
      stderrBytes: stderr.bytes, stderrOverflow: stderr.overflow,
      stderrTail: stderr.text.split(/\r?\n/).filter(Boolean).slice(-20),
    };
  } finally {
    await terminateMpv(child, ipc);
  }
}

export async function versionEvidence(paths, mpvIdentity = DEFAULT_MPV_IDENTITY) {
  const identity = validateMpvIdentity(mpvIdentity);
  const [mpv, ffmpeg, ffmpegLicense, mpvSha256] = await Promise.all([
    runCaptured(paths.mpv, ['--version'], { timeoutMs: 10_000 }),
    runCaptured(paths.ffmpeg, ['-version'], { timeoutMs: 10_000 }),
    runCaptured(paths.ffmpeg, ['-hide_banner', '-L'], { timeoutMs: 10_000 }),
    sha256File(paths.mpv),
  ]);
  const mpvLines = mpv.split(/\r?\n/).slice(0, 12);
  const versionMatched = mpvLines[0]?.includes(identity.versionToken) === true;
  const binaryMatched = mpvSha256 === identity.expectedSha256;
  return {
    mpv: mpvLines,
    ffmpeg: ffmpeg.split(/\r?\n/).slice(0, 12),
    ffmpegLicense: ffmpegLicense.split(/\r?\n/).filter(Boolean).slice(-12),
    mpvLicenseEvidence: 'mpv --version 不给出完整许可证文本；需结合固定资源清单审计',
    mpvSha256,
    mpvSourceContract: {
      status: versionMatched && binaryMatched ? 'matched' : 'unverified',
      versionMatched,
      binaryMatched,
      expectedSha256: identity.expectedSha256,
      ref: identity.sourceRef,
      url: identity.sourceUrl,
      evidence: '该提交按路径缓存 user hook、原位更新参数，并在播放中保留 renderer cache',
    },
  };
}

export function finalReportStatus(attempts) {
  if (attempts.length > 0 && attempts.every(({ status }) => status === 'passed')) return 'passed';
  if (attempts.some(({ status }) => status === 'failed')) return 'failed';
  if (attempts.some(({ status }) => status === 'unavailable')) return 'unavailable';
  if (attempts.some(({ status }) => status === 'unverified')) return 'unverified';
  return 'unavailable';
}

export function maximumValidatedResolutionClaim(status, attempts) {
  const gpuAttempts = attempts.filter(({ backend }) => backend !== 'cpu4');
  const sourceMatches = (attempt) => {
    const source = attempt.resolutionEvidence?.source;
    return Number.isSafeInteger(source?.expectedWidth) && source.expectedWidth > 0 &&
      Number.isSafeInteger(source?.expectedHeight) && source.expectedHeight > 0 &&
      source.actualWidth === source.expectedWidth && source.actualHeight === source.expectedHeight;
  };
  return status === 'passed' && gpuAttempts.length > 0 &&
    gpuAttempts.every((attempt) =>
      attempt.resolutionEvidence?.windowRenderTarget?.matched === true && sourceMatches(attempt)) &&
    gpuAttempts.some((attempt) =>
      attempt.resolutionEvidence?.source?.expectedWidth === 1920 &&
      attempt.resolutionEvidence.source.expectedHeight === 1080)
    ? '1920x1080'
    : null;
}

export async function runPhase1Gate({
  paths = DEFAULT_PATHS,
  updateCount = GPU_UPDATE_COUNT,
  mpvIdentity = DEFAULT_MPV_IDENTITY,
  fullscreenWindow = false,
} = {}) {
  if (process.platform !== 'win32') throw new Error('Phase 1 实机门禁只支持 Windows');
  if (!Number.isSafeInteger(updateCount) || updateCount <= 0) throw new Error('GPU 更新次数无效');
  if (typeof fullscreenWindow !== 'boolean') throw new Error('fullscreenWindow 必须是布尔值');
  const identity = validateMpvIdentity(mpvIdentity);
  await Promise.all([
    assertRegularFile(paths.ffmpeg, 'ffmpeg'),
    assertRegularFile(paths.mpv, 'mpv'),
    assertRegularFile(paths.shader, 'gpu83.hook'),
  ]);
  const temporaryRoot = await mkdtemp(join(tmpdir(), 'autolive-mpv-phase1-'));
  const report = {
    schemaVersion: 1,
    gate: 'mpv-libplacebo-phase1',
    startedAt: new Date().toISOString(),
    platform: { platform: process.platform, arch: process.arch, node: process.version },
    paths: { ffmpeg: resolve(paths.ffmpeg), mpv: resolve(paths.mpv), shader: resolve(paths.shader) },
    updateCount,
    samples: [],
    attempts: [],
    versions: null,
    status: 'failed',
  };
  try {
    report.versions = await versionEvidence(paths, identity);
    const sourceContractMatched = report.versions.mpvSourceContract.status === 'matched';
    const shaderSource = await readFile(paths.shader, 'utf8');
    const shaderParameters = parseShaderParameters(shaderSource);
    report.shader = {
      bytes: Buffer.byteLength(shaderSource),
      ...verifyShaderContract(shaderSource, shaderParameters),
    };
    report.samples = await generateSamples(paths.ffmpeg, temporaryRoot);
    for (const sample of report.samples) {
      for (const backend of ['d3d11', 'vulkan']) {
        report.attempts.push(await runMpvAttempt({
          mpvPath: paths.mpv, shaderPath: paths.shader, sample, backend, shaderParameters,
          sourceContractMatched, mpvIdentity: identity, updateCount,
          hiddenWindow: fullscreenWindow, fullscreenWindow,
        }));
      }
    }
    const cpuSample = report.samples.find(({ name }) => name === '1080p-60fps') ?? report.samples[0];
    report.attempts.push(await runCpu4Attempt({
      mpvPath: paths.mpv,
      sample: cpuSample,
      hiddenWindow: fullscreenWindow,
      fullscreenWindow,
    }));
    report.status = finalReportStatus(report.attempts);
    report.claims = {
      maximumValidatedResolution: maximumValidatedResolutionClaim(report.status, report.attempts),
      perFrameBudget:
        report.attempts.filter(({ backend }) => backend !== 'cpu4').every(({ frameBudgetEvidence }) => frameBudgetEvidence?.status === 'passed')
          ? 'passed'
          : 'not_passed',
      noRepeatedShaderCompilation:
        report.attempts.filter(({ backend }) => backend !== 'cpu4').every(({ shaderCompilationEvidence }) => shaderCompilationEvidence?.status === 'verified')
          ? 'verified'
          : 'unverified',
      operationalGpuUpdatesCompleted:
        report.attempts.filter(({ backend }) => backend !== 'cpu4').every(({ updateLatency }) => updateLatency?.count === updateCount),
    };
    report.finishedAt = new Date().toISOString();
    return report;
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

async function main() {
  const options = parseCliArgs(process.argv.slice(2));
  if (options.help) {
    console.log('用法: node desktop/tools/verify-mpv-phase1.mjs [--report <json文件>]');
    return;
  }
  let report;
  try {
    report = await runPhase1Gate();
  } catch (error) {
    report = {
      schemaVersion: 1,
      gate: 'mpv-libplacebo-phase1',
      status: /只支持 Windows|ENOENT|not found|无法找到/.test(error.message) ? 'unavailable' : 'failed',
      startedAt: new Date().toISOString(),
      finishedAt: new Date().toISOString(),
      error: error.message,
    };
  }
  const json = `${JSON.stringify(report, null, 2)}\n`;
  if (options.report) {
    await writeFile(options.report, json, { encoding: 'utf8', flag: 'wx' });
  }
  process.stdout.write(json);
  if (report.status !== 'passed') process.exitCode = 1;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  await main();
}
