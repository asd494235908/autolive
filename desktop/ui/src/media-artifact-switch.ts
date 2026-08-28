export type MediaArtifactIdentity = Readonly<{
  playbackGeneration: number;
  sourceRevision: number;
  planId: string;
  sequence: number;
}>;

export type MediaArtifactCandidate = MediaArtifactIdentity & Readonly<{
  targetAbsolutePositionMs: number;
  validUntilAbsolutePositionMs: number;
  outputDurationMs: number;
}>;

export type SourceMediaClock = Readonly<{
  loopIndex: number;
  positionMs: number;
}>;

function isSafeNonNegativeInteger(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function isValidIdentity(identity: MediaArtifactIdentity): boolean {
  return isSafeNonNegativeInteger(identity.playbackGeneration)
    && isSafeNonNegativeInteger(identity.sourceRevision)
    && typeof identity.planId === 'string'
    && identity.planId.trim().length > 0
    && Number.isSafeInteger(identity.sequence)
    && identity.sequence > 0;
}

export function mapAbsolutePositionToCandidateTime(
  absolutePositionMs: number,
  targetAbsolutePositionMs: number,
): number | null {
  if (
    !isSafeNonNegativeInteger(absolutePositionMs)
    || !isSafeNonNegativeInteger(targetAbsolutePositionMs)
    || absolutePositionMs < targetAbsolutePositionMs
  ) return null;
  const candidateTimeMs = absolutePositionMs - targetAbsolutePositionMs;
  return Number.isSafeInteger(candidateTimeMs) ? candidateTimeMs : null;
}

export function mapCandidateTimeToAbsolutePosition(
  candidateTimeMs: number,
  targetAbsolutePositionMs: number,
): number | null {
  if (
    !isSafeNonNegativeInteger(candidateTimeMs)
    || !isSafeNonNegativeInteger(targetAbsolutePositionMs)
  ) return null;
  const absolutePositionMs = targetAbsolutePositionMs + candidateTimeMs;
  return Number.isSafeInteger(absolutePositionMs) ? absolutePositionMs : null;
}

export function mapAbsolutePositionToSourceClock(
  absolutePositionMs: number,
  sourceDurationMs: number,
): SourceMediaClock | null {
  if (
    !isSafeNonNegativeInteger(absolutePositionMs)
    || !Number.isSafeInteger(sourceDurationMs)
    || sourceDurationMs <= 0
  ) return null;
  return {
    loopIndex: Math.floor(absolutePositionMs / sourceDurationMs),
    positionMs: absolutePositionMs % sourceDurationMs,
  };
}

export function isMediaArtifactIdentityCurrent(
  expected: MediaArtifactIdentity,
  current: MediaArtifactIdentity,
): boolean {
  return isValidIdentity(expected)
    && isValidIdentity(current)
    && expected.playbackGeneration === current.playbackGeneration
    && expected.sourceRevision === current.sourceRevision
    && expected.planId === current.planId
    && expected.sequence === current.sequence;
}

export function canSwitchToMediaArtifact(
  candidate: MediaArtifactCandidate,
  currentIdentity: MediaArtifactIdentity,
  absolutePositionMs: number,
  candidateMediaDurationMs: number,
  sourceDurationMs?: number,
  alignToSourceClock = false,
): boolean {
  return resolveMediaArtifactSwitchTime(
    candidate,
    currentIdentity,
    absolutePositionMs,
    candidateMediaDurationMs,
    sourceDurationMs,
    alignToSourceClock,
  ) !== null;
}

export function resolveMediaArtifactSwitchTime(
  candidate: MediaArtifactCandidate & Readonly<{ sourceStartMs?: number }>,
  currentIdentity: MediaArtifactIdentity,
  absolutePositionMs: number,
  candidateMediaDurationMs: number,
  sourceDurationMs?: number,
  alignToSourceClock = false,
): number | null {
  if (
    !isMediaArtifactIdentityCurrent(candidate, currentIdentity)
    || !isSafeNonNegativeInteger(candidate.targetAbsolutePositionMs)
    || !isSafeNonNegativeInteger(candidate.validUntilAbsolutePositionMs)
    || candidate.validUntilAbsolutePositionMs <= candidate.targetAbsolutePositionMs
    || !Number.isSafeInteger(candidate.outputDurationMs)
    || candidate.outputDurationMs <= 0
    || !Number.isSafeInteger(candidateMediaDurationMs)
    || candidateMediaDurationMs <= 0
    || !isSafeNonNegativeInteger(absolutePositionMs)
    || absolutePositionMs < candidate.targetAbsolutePositionMs
  ) return null;
  let candidateTimeMs = mapAbsolutePositionToCandidateTime(
    absolutePositionMs,
    candidate.targetAbsolutePositionMs,
  );
  if (alignToSourceClock) {
    const sourceClock = typeof sourceDurationMs === 'number'
      ? mapAbsolutePositionToSourceClock(absolutePositionMs, sourceDurationMs)
      : null;
    if (!sourceClock || typeof sourceDurationMs !== 'number') return null;
    const sourceStartMs = candidate.sourceStartMs ?? candidate.targetAbsolutePositionMs % sourceDurationMs;
    if (!isSafeNonNegativeInteger(sourceStartMs) || sourceStartMs >= sourceDurationMs) return null;
    candidateTimeMs = (
      sourceClock.positionMs - sourceStartMs + sourceDurationMs
    ) % sourceDurationMs;
  }
  return candidateTimeMs !== null
    && candidateTimeMs < candidate.outputDurationMs
    && candidateTimeMs < candidateMediaDurationMs
      ? candidateTimeMs
      : null;
}
