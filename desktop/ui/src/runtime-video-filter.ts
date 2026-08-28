export type RuntimeVideoFilterParameters = Readonly<{
  video_brightness_percent: number;
  video_contrast_percent: number;
  video_saturation_percent: number;
  video_hue_rotation_degrees: number;
  video_blur_radius_px: number;
  video_pixel_scale_percent: number;
  video_space_x_offset_px: number;
  video_space_y_offset_px: number;
  video_rotation_degrees: number;
  video_horizontal_flip_enabled: boolean;
  video_vertical_flip_enabled: boolean;
}>;

export type RuntimeVideoStyle = Readonly<{
  filter: string;
  transform: string;
}>;

const NEUTRAL_RUNTIME_VIDEO_STYLE: RuntimeVideoStyle = {
  filter: 'none',
  transform: 'none',
};

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}

function cssNumber(value: number): number {
  const rounded = Number(value.toFixed(4));
  return Object.is(rounded, -0) ? 0 : rounded;
}

export function buildRuntimeVideoStyle(
  params: RuntimeVideoFilterParameters | null,
  enabled: boolean,
): RuntimeVideoStyle {
  if (!enabled || !params) return NEUTRAL_RUNTIME_VIDEO_STYLE;
  const values = [
    params.video_brightness_percent,
    params.video_contrast_percent,
    params.video_saturation_percent,
    params.video_hue_rotation_degrees,
    params.video_blur_radius_px,
    params.video_pixel_scale_percent,
    params.video_space_x_offset_px,
    params.video_space_y_offset_px,
    params.video_rotation_degrees,
  ];
  if (!values.every(Number.isFinite)) return NEUTRAL_RUNTIME_VIDEO_STYLE;

  const brightness = cssNumber(100 + clamp(params.video_brightness_percent, -100, 100));
  const contrast = cssNumber(clamp(params.video_contrast_percent, 0, 200));
  const saturation = cssNumber(clamp(params.video_saturation_percent, 0, 200));
  const hue = cssNumber(clamp(params.video_hue_rotation_degrees, -180, 180));
  const blur = cssNumber(clamp(params.video_blur_radius_px, 0, 8));
  const scale = clamp(params.video_pixel_scale_percent, 95, 105) / 100;
  const scaleX = cssNumber(scale * (params.video_horizontal_flip_enabled ? -1 : 1));
  const scaleY = cssNumber(scale * (params.video_vertical_flip_enabled ? -1 : 1));
  const x = cssNumber(clamp(params.video_space_x_offset_px, -4, 4));
  const y = cssNumber(clamp(params.video_space_y_offset_px, -4, 4));
  const rotation = cssNumber(clamp(params.video_rotation_degrees, -180, 180));
  return {
    filter: `brightness(${brightness}%) contrast(${contrast}%) saturate(${saturation}%) hue-rotate(${hue}deg) blur(${blur}px)`,
    transform: `translate(${x}px, ${y}px) rotate(${rotation}deg) scale(${scaleX}, ${scaleY})`,
  };
}
