import type { PeriodRangeMs } from './runtime-parameter-scheduler';

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

export type LinkedMediaCycleSeed<AudioPayload, VideoPayload> = Readonly<{
  planId: string;
  periodMediaMs: number;
  audioPayload: AudioPayload;
  videoPayload: VideoPayload;
}>;

export type PeriodRangeIntersection =
  | Readonly<{ ok: true; range: PeriodRangeMs }>
  | Readonly<{ ok: false; reason: string }>;

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
): MediaCycleQueue<T> {
  assertSafeInteger('currentAbsolutePositionMs', currentAbsolutePositionMs, 0);
  assertSafeInteger('startSequence', startSequence, 1);
  validateSeed(seeds[0]);
  validateSeed(seeds[1]);
  const firstTarget = addMediaMs(currentAbsolutePositionMs, seeds[0].periodMediaMs);
  const first = createPlan(seeds[0], startSequence, firstTarget);
  const secondSequence = startSequence + 1;
  assertSafeInteger('sequence', secondSequence, 1);
  const second = createPlan(
    seeds[1],
    secondSequence,
    addMediaMs(firstTarget, seeds[1].periodMediaMs),
  );
  return [first, second];
}

/** N+2 原样晋升为 N+1，并在队尾补一个新的轻量计划。 */
export function advanceMediaCycleQueue<T>(
  queue: MediaCycleQueue<T>,
  nextSeed: MediaCycleSeed<T>,
): MediaCycleQueue<T> {
  const promoted = queue[1];
  validateSeed(nextSeed);
  const nextSequence = promoted.sequence + 1;
  assertSafeInteger('sequence', nextSequence, 1);
  const nextTarget = addMediaMs(promoted.targetAbsolutePositionMs, nextSeed.periodMediaMs);
  return [promoted, createPlan(nextSeed, nextSequence, nextTarget)];
}

export function createIndependentMediaCycleQueues<AudioPayload, VideoPayload>(
  currentAbsolutePositionMs: number,
  seeds: Readonly<{
    audio: readonly [MediaCycleSeed<AudioPayload>, MediaCycleSeed<AudioPayload>];
    video: readonly [MediaCycleSeed<VideoPayload>, MediaCycleSeed<VideoPayload>];
  }>,
  startSequence = 1,
): MediaCycleQueuePair<AudioPayload, VideoPayload> {
  return {
    audio: createMediaCycleQueue(currentAbsolutePositionMs, seeds.audio, startSequence),
    video: createMediaCycleQueue(currentAbsolutePositionMs, seeds.video, startSequence),
  };
}

export function createLinkedMediaCycleQueues<AudioPayload, VideoPayload>(
  currentAbsolutePositionMs: number,
  seeds: readonly [
    LinkedMediaCycleSeed<AudioPayload, VideoPayload>,
    LinkedMediaCycleSeed<AudioPayload, VideoPayload>,
  ],
  startSequence = 1,
): MediaCycleQueuePair<AudioPayload, VideoPayload> {
  const [first, second] = seeds;
  return {
    audio: createMediaCycleQueue(
      currentAbsolutePositionMs,
      [
        { planId: first.planId, periodMediaMs: first.periodMediaMs, payload: first.audioPayload },
        { planId: second.planId, periodMediaMs: second.periodMediaMs, payload: second.audioPayload },
      ],
      startSequence,
    ),
    video: createMediaCycleQueue(
      currentAbsolutePositionMs,
      [
        { planId: first.planId, periodMediaMs: first.periodMediaMs, payload: first.videoPayload },
        { planId: second.planId, periodMediaMs: second.periodMediaMs, payload: second.videoPayload },
      ],
      startSequence,
    ),
  };
}

export function intersectPeriodRanges(
  audio: PeriodRangeMs,
  video: PeriodRangeMs,
): PeriodRangeIntersection {
  if (
    !Number.isFinite(audio.minMs)
    || !Number.isFinite(audio.maxMs)
    || !Number.isFinite(video.minMs)
    || !Number.isFinite(video.maxMs)
    || audio.minMs > audio.maxMs
    || video.minMs > video.maxMs
  ) {
    return { ok: false, reason: '声音或视频周期范围无效' };
  }
  const minMs = Math.max(audio.minMs, video.minMs);
  const maxMs = Math.min(audio.maxMs, video.maxMs);
  return minMs <= maxMs
    ? { ok: true, range: { minMs, maxMs } }
    : { ok: false, reason: '声音与视频周期范围没有交集' };
}
