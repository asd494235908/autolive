export type MediaArtifactTimeline = Readonly<{
  planId: string;
  sequence: number;
  playbackGeneration: number;
  sourceRevision: number;
  targetAbsolutePositionMs: number;
  sourceStartMs: number;
  outputDurationMs: number;
  validUntilAbsolutePositionMs: number;
}>;

export function doesAudioWindowOverlapArtifact(input: Readonly<{
  sourceStartMs: number;
  outputDurationMs: number;
  sourceDurationMs: number;
  audioStartMs: number | null;
  audioEndMs: number | null;
  loopSource: boolean;
}>): boolean {
  const { audioStartMs, audioEndMs } = input;
  if (audioStartMs === null || audioEndMs === null) return true;
  if (
    !Number.isSafeInteger(audioStartMs)
    || !Number.isSafeInteger(audioEndMs)
    || !Number.isSafeInteger(input.sourceStartMs)
    || !Number.isSafeInteger(input.outputDurationMs)
    || !Number.isSafeInteger(input.sourceDurationMs)
    || audioStartMs < 0
    || audioEndMs <= audioStartMs
    || audioEndMs > input.sourceDurationMs
    || input.sourceStartMs < 0
    || input.sourceStartMs >= input.sourceDurationMs
    || input.outputDurationMs <= 0
  ) return true;

  const overlaps = (startMs: number, endMs: number) =>
    startMs < audioEndMs && endMs > audioStartMs;
  if (input.loopSource && input.outputDurationMs >= input.sourceDurationMs) return true;
  const candidateEndMs = input.sourceStartMs + input.outputDurationMs;
  if (candidateEndMs <= input.sourceDurationMs) {
    return overlaps(input.sourceStartMs, candidateEndMs);
  }
  if (!input.loopSource) {
    return overlaps(input.sourceStartMs, input.sourceDurationMs);
  }
  return overlaps(input.sourceStartMs, input.sourceDurationMs)
    || overlaps(0, candidateEndMs - input.sourceDurationMs);
}

function requireSafeInteger(name: string, value: number, minimum: number): void {
  if (!Number.isSafeInteger(value) || value < minimum) {
    throw new RangeError(`${name} must be a safe integer greater than or equal to ${minimum}`);
  }
}

export function createMediaArtifactTimeline(input: Readonly<{
  planId: string;
  sequence: number;
  playbackGeneration: number;
  sourceRevision: number;
  targetAbsolutePositionMs: number;
  validUntilAbsolutePositionMs: number;
  sourceDurationMs: number;
  safetyTailMs: number;
}>): MediaArtifactTimeline {
  if (!input.planId.trim()) throw new RangeError('planId must not be empty');
  requireSafeInteger('sequence', input.sequence, 1);
  requireSafeInteger('playbackGeneration', input.playbackGeneration, 0);
  requireSafeInteger('sourceRevision', input.sourceRevision, 0);
  requireSafeInteger('targetAbsolutePositionMs', input.targetAbsolutePositionMs, 0);
  requireSafeInteger('validUntilAbsolutePositionMs', input.validUntilAbsolutePositionMs, 1);
  requireSafeInteger('sourceDurationMs', input.sourceDurationMs, 1);
  requireSafeInteger('safetyTailMs', input.safetyTailMs, 0);
  if (input.validUntilAbsolutePositionMs <= input.targetAbsolutePositionMs) {
    throw new RangeError('validUntilAbsolutePositionMs must be after targetAbsolutePositionMs');
  }
  const outputDurationMs = input.validUntilAbsolutePositionMs
    - input.targetAbsolutePositionMs
    + input.safetyTailMs;
  requireSafeInteger('outputDurationMs', outputDurationMs, 1);
  return {
    planId: input.planId,
    sequence: input.sequence,
    playbackGeneration: input.playbackGeneration,
    sourceRevision: input.sourceRevision,
    targetAbsolutePositionMs: input.targetAbsolutePositionMs,
    sourceStartMs: input.targetAbsolutePositionMs % input.sourceDurationMs,
    outputDurationMs,
    validUntilAbsolutePositionMs: input.validUntilAbsolutePositionMs,
  };
}
