export type RuntimeBaseParameters = {
  audio_gain_db: number;
  video_brightness_percent: number;
  video_contrast_percent: number;
  video_saturation_percent: number;
  video_hue_rotation_degrees: number;
  video_blur_radius_px: number;
  video_pixel_scale_percent: number;
  video_space_x_offset_px: number;
  video_space_y_offset_px: number;
};

export type RuntimePreviewParameters = RuntimeBaseParameters;

const DEFAULT_RUNTIME_VARIATION_PERIOD_MS = 5_000;
const MIN_RUNTIME_VARIATION_PERIOD_MS = 500;
const MAX_RUNTIME_VARIATION_PERIOD_MS = 60_000;

const AUDIO_GAIN_MIN = -6;
const AUDIO_GAIN_MAX = 6;
const VIDEO_BRIGHTNESS_MIN = -100;
const VIDEO_BRIGHTNESS_MAX = 100;
const VIDEO_CONTRAST_MIN = 0;
const VIDEO_CONTRAST_MAX = 200;
const VIDEO_SATURATION_MIN = 0;
const VIDEO_SATURATION_MAX = 200;
const VIDEO_HUE_MIN = -180;
const VIDEO_HUE_MAX = 180;
const VIDEO_BLUR_MIN = 0;
const VIDEO_BLUR_MAX = 8;
const VIDEO_PIXEL_SCALE_MIN = 95;
const VIDEO_PIXEL_SCALE_MAX = 105;
const VIDEO_SPACE_OFFSET_MIN = -4;
const VIDEO_SPACE_OFFSET_MAX = 4;

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}

function finiteOrZero(value: number): number {
  return Number.isFinite(value) ? value : 0;
}

function cycleWave(cycle: number, phase: number): number {
  return Math.sin(cycle * 1.61803398875 + phase);
}

function previewValue(
  baseValue: number,
  minimum: number,
  maximum: number,
  cycle: number,
  amplitude: number,
  phase: number,
): number {
  const safeBaseValue = finiteOrZero(baseValue);
  if (safeBaseValue < minimum || safeBaseValue > maximum) {
    return clamp(safeBaseValue, minimum, maximum);
  }

  return clamp(safeBaseValue + cycleWave(cycle, phase) * amplitude, minimum, maximum);
}

export function normalizeRuntimeVariationPeriod(periodMs?: number | null): number {
  if (!Number.isFinite(periodMs as number) || (periodMs as number) <= 0) {
    return DEFAULT_RUNTIME_VARIATION_PERIOD_MS;
  }

  if ((periodMs as number) < MIN_RUNTIME_VARIATION_PERIOD_MS) {
    return MIN_RUNTIME_VARIATION_PERIOD_MS;
  }

  return clamp(
    periodMs as number,
    MIN_RUNTIME_VARIATION_PERIOD_MS,
    MAX_RUNTIME_VARIATION_PERIOD_MS,
  );
}

export function isRuntimeVariationDue(
  nowMs: number,
  lastChangeMs: number,
  periodMs?: number | null,
): boolean {
  if (!Number.isFinite(nowMs) || !Number.isFinite(lastChangeMs)) {
    return false;
  }

  const effectivePeriodMs = normalizeRuntimeVariationPeriod(periodMs);
  return nowMs - lastChangeMs >= effectivePeriodMs;
}

export function buildRuntimePreviewParameters(
  baseParameters: RuntimeBaseParameters,
  cycle: number,
): RuntimePreviewParameters {
  const safeCycle = Number.isFinite(cycle) ? cycle : 0;

  return {
    audio_gain_db: previewValue(baseParameters.audio_gain_db, AUDIO_GAIN_MIN, AUDIO_GAIN_MAX, safeCycle, 1.5, 0),
    video_brightness_percent: previewValue(
      baseParameters.video_brightness_percent,
      VIDEO_BRIGHTNESS_MIN,
      VIDEO_BRIGHTNESS_MAX,
      safeCycle,
      12,
      0.3,
    ),
    video_contrast_percent: previewValue(
      baseParameters.video_contrast_percent,
      VIDEO_CONTRAST_MIN,
      VIDEO_CONTRAST_MAX,
      safeCycle,
      10,
      1.1,
    ),
    video_saturation_percent: previewValue(
      baseParameters.video_saturation_percent,
      VIDEO_SATURATION_MIN,
      VIDEO_SATURATION_MAX,
      safeCycle,
      10,
      2.2,
    ),
    video_hue_rotation_degrees: previewValue(
      baseParameters.video_hue_rotation_degrees,
      VIDEO_HUE_MIN,
      VIDEO_HUE_MAX,
      safeCycle,
      12,
      2.8,
    ),
    video_blur_radius_px: previewValue(
      baseParameters.video_blur_radius_px,
      VIDEO_BLUR_MIN,
      VIDEO_BLUR_MAX,
      safeCycle,
      1,
      0.7,
    ),
    video_pixel_scale_percent: previewValue(
      baseParameters.video_pixel_scale_percent,
      VIDEO_PIXEL_SCALE_MIN,
      VIDEO_PIXEL_SCALE_MAX,
      safeCycle,
      2,
      1.7,
    ),
    video_space_x_offset_px: previewValue(
      baseParameters.video_space_x_offset_px,
      VIDEO_SPACE_OFFSET_MIN,
      VIDEO_SPACE_OFFSET_MAX,
      safeCycle,
      2,
      2.5,
    ),
    video_space_y_offset_px: previewValue(
      baseParameters.video_space_y_offset_px,
      VIDEO_SPACE_OFFSET_MIN,
      VIDEO_SPACE_OFFSET_MAX,
      safeCycle,
      2,
      3.4,
    ),
  };
}
