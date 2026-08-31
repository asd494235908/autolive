import { spawn } from 'node:child_process';
import { resolve } from 'node:path';

const ANALYSIS_WIDTH = 320;
const ANALYSIS_HEIGHT = 180;
const CHANNELS = 3;
const FRAME_BYTES = ANALYSIS_WIDTH * ANALYSIS_HEIGHT * CHANNELS;
const MAX_STDERR_BYTES = 64 * 1024;

function safeScreenshotPath(directory, index) {
  const root = resolve(directory);
  const path = resolve(root, `frame-${String(index).padStart(4, '0')}.png`);
  if (!path.startsWith(`${root}\\`) && !path.startsWith(`${root}/`)) {
    throw new Error('截图路径逃逸候选验收临时目录');
  }
  return path;
}

export function serializeShaderOptions(options) {
  if (options === null || typeof options !== 'object' || Array.isArray(options)) {
    throw new Error('shader 参数快照必须是对象');
  }
  return Object.entries(options)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, value]) => {
      if (!/^al_[A-Za-z0-9_]+$/.test(name)) throw new Error(`不安全的 shader 参数：${name}`);
      if (typeof value !== 'number' || !Number.isFinite(value)) throw new Error(`${name} 必须是有限数`);
      return `${name}=${value}`;
    })
    .join(',');
}

async function readFiniteMediaPts(ipc, stage) {
  const pts = (await ipc.send(['get_property', 'time-pos/full'])).data;
  if (typeof pts !== 'number' || !Number.isFinite(pts) || pts < 0) {
    throw new Error(`${stage} 缺少有限的暂停媒体 PTS`);
  }
  return pts;
}

export async function capturePausedScenarioFrames({ ipc, directory, scenarios }) {
  if (!Array.isArray(scenarios) || scenarios.length === 0) throw new Error('逐字段场景不能为空');
  const originalPause = (await ipc.send(['get_property', 'pause'])).data === true;
  const captures = [];
  await ipc.send(['set_property', 'pause', true]);
  try {
    const anchorPtsSeconds = await readFiniteMediaPts(ipc, '截图锚点');
    for (let scenarioIndex = 0; scenarioIndex < scenarios.length; scenarioIndex += 1) {
      const scenario = scenarios[scenarioIndex];
      if (!scenario || typeof scenario.field !== 'string') throw new Error('逐字段场景缺少 field');
      const mode = scenario.mode ?? 'pair_difference';
      const states = scenario.states ?? [
        { role: 'neutral', options: scenario.neutralOptions },
        { role: 'active', options: scenario.activeOptions },
      ];
      if (!Array.isArray(states) || states.length < 2 || states.length > 3) {
        throw new Error(`${scenario.field} 必须声明 2 或 3 档截图状态`);
      }
      for (const state of states) {
        if (!state || typeof state.role !== 'string' || state.role.length === 0) {
          throw new Error(`${scenario.field} 截图状态缺少 role`);
        }
        const path = safeScreenshotPath(directory, captures.length);
        await ipc.send(['set_property', 'glsl-shader-opts', serializeShaderOptions(state.options)]);
        await ipc.send(['screenshot-to-file', path, 'video']);
        const mediaPtsSeconds = await readFiniteMediaPts(ipc, `${scenario.field}/${state.role}`);
        if (Math.abs(mediaPtsSeconds - anchorPtsSeconds) > 1e-9) {
          throw new Error(
            `${scenario.field}/${state.role} 截图跨帧：锚点 ${anchorPtsSeconds}s，实际 ${mediaPtsSeconds}s`,
          );
        }
        captures.push({
          scenarioIndex,
          field: scenario.field,
          mode,
          role: state.role,
          path,
          mediaPtsSeconds,
        });
      }
    }
  } finally {
    await ipc.send(['set_property', 'pause', originalPause]);
  }
  return captures;
}

function collectRawFrames(ffmpegPath, inputPattern, frameCount, timeoutMs = 60_000) {
  if (!Number.isSafeInteger(frameCount) || frameCount <= 0 || frameCount > 256) {
    throw new Error('截图帧数量无效');
  }
  const expectedBytes = frameCount * FRAME_BYTES;
  const argumentsList = [
    '-hide_banner', '-loglevel', 'error', '-framerate', '1', '-start_number', '0',
    '-i', inputPattern,
    '-vf', `scale=${ANALYSIS_WIDTH}:${ANALYSIS_HEIGHT}:flags=area,format=rgb24`,
    '-frames:v', String(frameCount), '-f', 'rawvideo', 'pipe:1',
  ];
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(ffmpegPath, argumentsList, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    const stdout = [];
    const stderr = [];
    let stdoutBytes = 0;
    let stderrBytes = 0;
    let overflow = false;
    child.stdout.on('data', (chunk) => {
      stdoutBytes += chunk.length;
      if (stdoutBytes <= expectedBytes) stdout.push(chunk);
      else overflow = true;
    });
    child.stderr.on('data', (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes <= MAX_STDERR_BYTES) stderr.push(chunk);
      else overflow = true;
    });
    const timer = setTimeout(() => {
      child.kill();
      rejectPromise(new Error('FFmpeg 截图差异分析超时'));
    }, timeoutMs);
    child.once('error', (error) => {
      clearTimeout(timer);
      rejectPromise(error);
    });
    child.once('exit', (code, signal) => {
      clearTimeout(timer);
      const errorText = Buffer.concat(stderr).toString('utf8');
      if (overflow || stdoutBytes !== expectedBytes) {
        rejectPromise(new Error(`FFmpeg 截图差异输出大小异常：${stdoutBytes}/${expectedBytes}`));
      } else if (code !== 0) {
        rejectPromise(new Error(`FFmpeg 截图差异分析退出码 ${code ?? signal}：${errorText.slice(-2000)}`));
      } else {
        resolvePromise(Buffer.concat(stdout));
      }
    });
  });
}

export function compareRawRgbPairs(rawFrames, fields, {
  width = ANALYSIS_WIDTH,
  height = ANALYSIS_HEIGHT,
  minimumChangedPixelRatio = 0.0001,
  minimumMeanAbsoluteDelta = 0.001,
  } = {}) {
  if (!Buffer.isBuffer(rawFrames)) throw new Error('RGB 帧数据必须是 Buffer');
  if (!Array.isArray(fields) || fields.length === 0) throw new Error('字段列表不能为空');
  const frameBytes = width * height * CHANNELS;
  if (rawFrames.length !== fields.length * 2 * frameBytes) throw new Error('RGB 帧数量与字段场景不匹配');
  return fields.map((field, pairIndex) => {
    const difference = compareFrames(rawFrames, pairIndex * 2, pairIndex * 2 + 1, frameBytes, width, height);
    return {
      field,
      status: difference.changedPixelRatio >= minimumChangedPixelRatio
          && difference.meanAbsoluteDelta >= minimumMeanAbsoluteDelta
          ? 'passed'
          : 'failed',
      ...difference,
    };
  });
}

function compareFrames(rawFrames, leftIndex, rightIndex, frameBytes, width, height) {
  const leftOffset = leftIndex * frameBytes;
  const rightOffset = rightIndex * frameBytes;
  let absoluteDelta = 0;
  let maximumChannelDelta = 0;
  let changedPixels = 0;
  let oneCodeValueChangedPixels = 0;
  for (let pixel = 0; pixel < width * height; pixel += 1) {
    let pixelChanged = false;
    let oneCodeValuePixelChanged = false;
    for (let channel = 0; channel < CHANNELS; channel += 1) {
      const offset = pixel * CHANNELS + channel;
      const delta = Math.abs(rawFrames[leftOffset + offset] - rawFrames[rightOffset + offset]);
      absoluteDelta += delta;
      maximumChannelDelta = Math.max(maximumChannelDelta, delta);
      if (delta >= 1) oneCodeValuePixelChanged = true;
      if (delta >= 2) pixelChanged = true;
    }
    if (oneCodeValuePixelChanged) oneCodeValueChangedPixels += 1;
    if (pixelChanged) changedPixels += 1;
  }
  return {
    changedPixels,
    changedPixelRatio: changedPixels / (width * height),
    oneCodeValueChangedPixels,
    oneCodeValueChangedPixelRatio: oneCodeValueChangedPixels / (width * height),
    meanAbsoluteDelta: absoluteDelta / frameBytes,
    maximumChannelDelta,
  };
}

function groupCaptures(captures) {
  const groups = [];
  for (const [index, capture] of captures.entries()) {
    if (!capture || typeof capture.field !== 'string' || typeof capture.role !== 'string') {
      throw new Error('截图证据缺少 field/role');
    }
    const scenarioIndex = Number.isSafeInteger(capture.scenarioIndex)
      ? capture.scenarioIndex
      : groups.length === 0 || groups.at(-1).field !== capture.field
        ? groups.length
        : groups.at(-1).scenarioIndex;
    let group = groups.at(-1);
    if (!group || group.scenarioIndex !== scenarioIndex) {
      group = { scenarioIndex, field: capture.field, mode: capture.mode ?? 'pair_difference', captures: [] };
      groups.push(group);
    }
    if (group.field !== capture.field || group.mode !== (capture.mode ?? 'pair_difference')) {
      throw new Error('同一场景的截图 field/mode 不一致');
    }
    group.captures.push({ capture, index });
  }
  return groups;
}

export function compareRawRgbScenarios(rawFrames, captures, {
  width = ANALYSIS_WIDTH,
  height = ANALYSIS_HEIGHT,
  minimumChangedPixelRatio = 0.0001,
  minimumMeanAbsoluteDelta = 0.001,
  maximumSameMeanAbsoluteDelta = 0.01,
  maximumSameChannelDelta = 1,
} = {}) {
  if (!Buffer.isBuffer(rawFrames)) throw new Error('RGB 帧数据必须是 Buffer');
  if (!Array.isArray(captures) || captures.length === 0) throw new Error('截图证据不能为空');
  const frameBytes = width * height * CHANNELS;
  if (rawFrames.length !== captures.length * frameBytes) throw new Error('RGB 帧数量与截图证据不匹配');

  return groupCaptures(captures).map(({ field, mode, captures: group }) => {
    const roles = group.map(({ capture }) => capture.role).join(',');
    const indexes = group.map(({ index }) => index);
    if (mode === 'product_progression' || mode === 'product_response') {
      if (roles !== 'baseline,low,active') throw new Error(`${field} 产品截图必须是 baseline/low/active`);
      const baselineToLow = compareFrames(rawFrames, indexes[0], indexes[1], frameBytes, width, height);
      const baselineToActive = compareFrames(rawFrames, indexes[0], indexes[2], frameBytes, width, height);
      const lowToActive = compareFrames(rawFrames, indexes[1], indexes[2], frameBytes, width, height);
      const lowMeasured = baselineToLow.oneCodeValueChangedPixelRatio >= minimumChangedPixelRatio;
      const activeMeasured = baselineToActive.changedPixelRatio >= minimumChangedPixelRatio
        && baselineToActive.meanAbsoluteDelta >= minimumMeanAbsoluteDelta;
      const lowBelowActive = baselineToLow.meanAbsoluteDelta < baselineToActive.meanAbsoluteDelta;
      const responseSeparated = lowToActive.changedPixelRatio >= minimumChangedPixelRatio
        && lowToActive.meanAbsoluteDelta >= minimumMeanAbsoluteDelta;
      return {
        field,
        mode,
        status: lowMeasured && activeMeasured
          && (mode === 'product_response' ? responseSeparated : lowBelowActive) ? 'passed' : 'failed',
        baselineToLow,
        baselineToActive,
        lowToActive,
        lowMeasured,
        activeMeasured,
        lowBelowActive,
        responseSeparated,
      };
    }

    if (group.length !== 2) throw new Error(`${field} ${mode} 截图必须恰好两档`);
    const difference = compareFrames(rawFrames, indexes[0], indexes[1], frameBytes, width, height);
    if (mode === 'product_switch' && roles === 'baseline,active') {
      return {
        field,
        mode,
        status: difference.changedPixelRatio >= minimumChangedPixelRatio
          && difference.meanAbsoluteDelta >= minimumMeanAbsoluteDelta ? 'passed' : 'failed',
        ...difference,
      };
    }
    if (mode === 'expected_same') {
      return {
        field,
        mode,
        status: difference.maximumChannelDelta <= maximumSameChannelDelta
          && difference.meanAbsoluteDelta <= maximumSameMeanAbsoluteDelta ? 'passed' : 'failed',
        ...difference,
      };
    }
    if (mode !== 'pair_difference' || roles !== 'neutral,active') {
      throw new Error(`${field} 未知截图分析模式或角色顺序: ${mode}/${roles}`);
    }
    return {
      field,
      mode,
      status: difference.changedPixelRatio >= minimumChangedPixelRatio
        && difference.meanAbsoluteDelta >= minimumMeanAbsoluteDelta ? 'passed' : 'failed',
      ...difference,
    };
  });
}

export async function analyzeScenarioFrames({ ffmpegPath, directory, captures }) {
  if (!Array.isArray(captures) || captures.length === 0) throw new Error('截图证据不能为空');
  const inputPattern = resolve(directory, 'frame-%04d.png');
  const rawFrames = await collectRawFrames(ffmpegPath, inputPattern, captures.length);
  const comparisons = compareRawRgbScenarios(rawFrames, captures);
  return {
    status: comparisons.every(({ status }) => status === 'passed') ? 'passed' : 'failed',
    analysisSize: { width: ANALYSIS_WIDTH, height: ANALYSIS_HEIGHT },
    comparisons,
  };
}
