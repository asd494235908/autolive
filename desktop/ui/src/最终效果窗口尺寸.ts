type FinalEffectWindowResizeInput = {
  width: number | null | undefined;
  height: number | null | undefined;
  sourcePath?: string | null;
  videoReference?: string | null;
};

export function buildFinalEffectWindowResizeKey({
  width,
  height,
  sourcePath,
  videoReference,
}: FinalEffectWindowResizeInput): string | null {
  if (
    typeof width !== 'number' ||
    typeof height !== 'number' ||
    !Number.isSafeInteger(width) ||
    !Number.isSafeInteger(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return null;
  }

  return `${videoReference ?? sourcePath ?? 'source'}:${width}x${height}`;
}
