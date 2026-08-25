type FinalEffectWindowResizeInput = {
  playbackGeneration: number | null | undefined;
  width: number | null | undefined;
  height: number | null | undefined;
  sourcePath: string | null | undefined;
};

type FinalEffectMediaIdentityInput = {
  playbackGeneration: number | null | undefined;
  sourceIdentity: string | null | undefined;
};

export function buildFinalEffectMediaIdentity({
  playbackGeneration,
  sourceIdentity,
}: FinalEffectMediaIdentityInput): string | null {
  if (
    typeof playbackGeneration !== 'number'
    || !Number.isSafeInteger(playbackGeneration)
    || playbackGeneration < 0
    || typeof sourceIdentity !== 'string'
    || sourceIdentity.trim().length === 0
  ) {
    return null;
  }

  return JSON.stringify([playbackGeneration, sourceIdentity]);
}

export function isFinalEffectMediaIdentityCurrent(
  expectedIdentity: string | null,
  current: FinalEffectMediaIdentityInput,
): boolean {
  return expectedIdentity !== null && expectedIdentity === buildFinalEffectMediaIdentity(current);
}

export function buildFinalEffectWindowResizeKey({
  playbackGeneration,
  width,
  height,
  sourcePath,
}: FinalEffectWindowResizeInput): string | null {
  const mediaIdentity = buildFinalEffectMediaIdentity({
    playbackGeneration,
    sourceIdentity: sourcePath,
  });
  if (
    !mediaIdentity ||
    typeof width !== 'number' ||
    typeof height !== 'number' ||
    !Number.isSafeInteger(width) ||
    !Number.isSafeInteger(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return null;
  }

  // processed 路径就绪不改变播放项身份；换项时即使同尺寸也重新确认窗口表面。
  return `${mediaIdentity}:${width}x${height}`;
}
