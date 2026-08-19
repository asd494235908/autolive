type FinalEffectWindowResizeInput = {
  width: number | null | undefined;
  height: number | null | undefined;
  sourcePath?: string | null;
  videoReference?: string | null;
};

export function buildFinalEffectWindowResizeKey({
  width,
  height,
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

  // ponytail: 只跟分辨率走；processed 路径变绿不能触发 center
  return `${width}x${height}`;
}
