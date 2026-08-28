export type MediaCycleSeed<T> = Readonly<{
  planId: string;
  periodMediaMs: number;
  payload: T;
}>;

export type MediaCyclePlan<T> = MediaCycleSeed<T> & Readonly<{
  sequence: number;
  targetAbsolutePositionMs: number;
}>;

export type MediaCycleQueue<T> = readonly [MediaCyclePlan<T>, MediaCyclePlan<T>];

export type MediaCycleQueuePair<AudioPayload, VideoPayload> = Readonly<{
  audio: MediaCycleQueue<AudioPayload>;
  video: MediaCycleQueue<VideoPayload>;
}>;

export type MediaCycleTargetAligner = (targetAbsolutePositionMs: number) => number;

type SourceBoundedVideoBoundaryTarget = Readonly<{
  targetAbsolutePositionMs: number;
  endsAtSourceBoundary: boolean;
}>;

export type SourceBoundedVideoCycleTarget = Readonly<{
  targetAbsolutePositionMs: number;
  skipVideoProcessing: boolean;
}>;

export type SourceBoundedVideoCycleQueueTargets = readonly [
  SourceBoundedVideoCycleTarget,
  SourceBoundedVideoCycleTarget,
];

function assertSafeInteger(name: string, value: number, minimum: number): void {
  if (!Number.isSafeInteger(value) || value < minimum) {
    throw new RangeError(`${name} must be a safe integer greater than or equal to ${minimum}`);
  }
}

function validateSeed<T>(seed: MediaCycleSeed<T>): void {
  if (!seed.planId.trim()) throw new RangeError('planId must not be empty');
  assertSafeInteger('periodMediaMs', seed.periodMediaMs, 1);
}

function addMediaMs(left: number, right: number): number {
  const result = left + right;
  assertSafeInteger('absolute media position', result, 0);
  return result;
}

function alignMediaTarget(
  targetAbsolutePositionMs: number,
  alignTarget?: MediaCycleTargetAligner,
): number {
  if (!alignTarget) return targetAbsolutePositionMs;
  const aligned = alignTarget(targetAbsolutePositionMs);
  assertSafeInteger('aligned absolute media position', aligned, targetAbsolutePositionMs);
  return aligned;
}

/** 将整数毫秒目标投影到不早于该目标的最近视频帧。 */
export function alignMediaPositionToVideoFrame(
  targetAbsolutePositionMs: number,
  frameRateFps: number | null | undefined,
): number {
  assertSafeInteger('targetAbsolutePositionMs', targetAbsolutePositionMs, 0);
  if (
    typeof frameRateFps !== 'number'
    || !Number.isFinite(frameRateFps)
    || frameRateFps < 1
    || frameRateFps > 240
  ) {
    return targetAbsolutePositionMs;
  }
  const framePosition = targetAbsolutePositionMs * frameRateFps / 1_000;
  const frameIndex = Math.ceil(framePosition - 1e-9);
  assertSafeInteger('video frame index', frameIndex, 0);
  const aligned = Math.round(frameIndex * 1_000 / frameRateFps);
  return Math.max(targetAbsolutePositionMs, aligned);
}

/** 单项循环视频的周期目标不能跨过下一次源 EOF；EOF 边界只承载 Original。 */
function resolveSourceBoundedVideoCycleTarget(
  currentAbsolutePositionMs: number,
  requestedTargetAbsolutePositionMs: number,
  sourceDurationMs: number,
): SourceBoundedVideoBoundaryTarget {
  assertSafeInteger('currentAbsolutePositionMs', currentAbsolutePositionMs, 0);
  assertSafeInteger('requestedTargetAbsolutePositionMs', requestedTargetAbsolutePositionMs, 0);
  assertSafeInteger('sourceDurationMs', sourceDurationMs, 1);
  if (requestedTargetAbsolutePositionMs <= currentAbsolutePositionMs) {
    throw new RangeError('requestedTargetAbsolutePositionMs must be greater than currentAbsolutePositionMs');
  }
  const nextSourceBoundaryMs = (Math.floor(currentAbsolutePositionMs / sourceDurationMs) + 1)
    * sourceDurationMs;
  assertSafeInteger('next source boundary', nextSourceBoundaryMs, currentAbsolutePositionMs + 1);
  return requestedTargetAbsolutePositionMs >= nextSourceBoundaryMs
    ? { targetAbsolutePositionMs: nextSourceBoundaryMs, endsAtSourceBoundary: true }
    : { targetAbsolutePositionMs: requestedTargetAbsolutePositionMs, endsAtSourceBoundary: false };
}

/** 参数作用于当前目标到下一目标，因此是否为源尾段由后一目标是否命中 EOF 决定。 */
export function resolveSourceBoundedVideoCycleQueueTargets(
  currentAbsolutePositionMs: number,
  firstPeriodMediaMs: number,
  secondPeriodMediaMs: number,
  sourceDurationMs: number,
  alignTarget?: MediaCycleTargetAligner,
): SourceBoundedVideoCycleQueueTargets {
  assertSafeInteger('firstPeriodMediaMs', firstPeriodMediaMs, 1);
  assertSafeInteger('secondPeriodMediaMs', secondPeriodMediaMs, 1);
  const first = resolveSourceBoundedVideoCycleTarget(
    currentAbsolutePositionMs,
    alignMediaTarget(addMediaMs(currentAbsolutePositionMs, firstPeriodMediaMs), alignTarget),
    sourceDurationMs,
  );
  const second = resolveSourceBoundedVideoCycleTarget(
    first.targetAbsolutePositionMs,
    alignMediaTarget(addMediaMs(first.targetAbsolutePositionMs, secondPeriodMediaMs), alignTarget),
    sourceDurationMs,
  );
  return [
    {
      targetAbsolutePositionMs: first.targetAbsolutePositionMs,
      skipVideoProcessing: second.endsAtSourceBoundary,
    },
    {
      targetAbsolutePositionMs: second.targetAbsolutePositionMs,
      skipVideoProcessing: false,
    },
  ];
}

function createPlan<T>(
  seed: MediaCycleSeed<T>,
  sequence: number,
  targetAbsolutePositionMs: number,
): MediaCyclePlan<T> {
  validateSeed(seed);
  assertSafeInteger('sequence', sequence, 1);
  return { ...seed, sequence, targetAbsolutePositionMs };
}

/** 给两个轻量 payload 盖上 N+1/N+2 绝对媒体时间目标。 */
export function createMediaCycleQueue<T>(
  currentAbsolutePositionMs: number,
  seeds: readonly [MediaCycleSeed<T>, MediaCycleSeed<T>],
  startSequence = 1,
  alignTarget?: MediaCycleTargetAligner,
): MediaCycleQueue<T> {
  assertSafeInteger('currentAbsolutePositionMs', currentAbsolutePositionMs, 0);
  assertSafeInteger('startSequence', startSequence, 1);
  validateSeed(seeds[0]);
  validateSeed(seeds[1]);
  const firstTarget = alignMediaTarget(
    addMediaMs(currentAbsolutePositionMs, seeds[0].periodMediaMs),
    alignTarget,
  );
  const first = createPlan(seeds[0], startSequence, firstTarget);
  const secondSequence = startSequence + 1;
  assertSafeInteger('sequence', secondSequence, 1);
  const second = createPlan(
    seeds[1],
    secondSequence,
    alignMediaTarget(addMediaMs(firstTarget, seeds[1].periodMediaMs), alignTarget),
  );
  return [first, second];
}

/** N+2 原样晋升为 N+1，并在队尾补一个新的轻量计划。 */
export function advanceMediaCycleQueue<T>(
  queue: MediaCycleQueue<T>,
  nextSeed: MediaCycleSeed<T>,
  alignTarget?: MediaCycleTargetAligner,
): MediaCycleQueue<T> {
  const promoted = queue[1];
  validateSeed(nextSeed);
  const nextSequence = promoted.sequence + 1;
  assertSafeInteger('sequence', nextSequence, 1);
  const nextTarget = alignMediaTarget(
    addMediaMs(promoted.targetAbsolutePositionMs, nextSeed.periodMediaMs),
    alignTarget,
  );
  return [promoted, createPlan(nextSeed, nextSequence, nextTarget)];
}

/** 将同一条 N+1 的绝对媒体目标投影为 0–100；暂停或缓冲时复用最后位置即可自然冻结。 */
export function getMediaCycleProgressPercent(
  plan: MediaCyclePlan<unknown> | null | undefined,
  absolutePositionMs: number | null | undefined,
): number {
  if (
    !plan
    || typeof absolutePositionMs !== 'number'
    || !Number.isSafeInteger(absolutePositionMs)
    || absolutePositionMs < 0
    || !Number.isSafeInteger(plan.periodMediaMs)
    || plan.periodMediaMs <= 0
    || !Number.isSafeInteger(plan.targetAbsolutePositionMs)
  ) return 0;
  const elapsedMs = absolutePositionMs - (plan.targetAbsolutePositionMs - plan.periodMediaMs);
  return Math.max(0, Math.min(100, Math.floor(elapsedMs / plan.periodMediaMs * 100)));
}


export function createIndependentMediaCycleQueues<AudioPayload, VideoPayload>(
  currentAbsolutePositionMs: number,
  seeds: Readonly<{
    audio: readonly [MediaCycleSeed<AudioPayload>, MediaCycleSeed<AudioPayload>];
    video: readonly [MediaCycleSeed<VideoPayload>, MediaCycleSeed<VideoPayload>];
  }>,
  startSequence = 1,
  targetAligners: Readonly<{
    audio?: MediaCycleTargetAligner;
    video?: MediaCycleTargetAligner;
  }> = {},
): MediaCycleQueuePair<AudioPayload, VideoPayload> {
  return {
    audio: createMediaCycleQueue(
      currentAbsolutePositionMs,
      seeds.audio,
      startSequence,
      targetAligners.audio,
    ),
    video: createMediaCycleQueue(
      currentAbsolutePositionMs,
      seeds.video,
      startSequence,
      targetAligners.video,
    ),
  };
}
