export type RuntimeBaseParameters = {
  audio_gain_db: number;
  audio_low_eq_db: number;
  audio_mid_eq_db: number;
  audio_high_eq_db: number;
  /** 用户配置的基础声音参数，供最终效果窗口实时预览/显示。 */
  audio_input_gain_db?: number;
  audio_output_gain_db?: number;
  audio_loudness_adjustment_db?: number;
  audio_pitch_shift_semitones?: number;
  audio_playback_speed?: number;
  audio_fade_in_ms?: number;
  audio_fade_out_ms?: number;
  audio_reverb_wet_percent?: number;
  audio_noise_reduction_percent?: number;
  audio_phase_perturbation_percent?: number;
  audio_vibrato_frequency_hz?: number;
  audio_vibrato_depth_percent?: number;
  audio_environment_noise_percent?: number;
  audio_environment_noise_dbfs?: number;
  audio_filter_q?: number;
  audio_sample_rate_hz?: number;
  audio_output_bitrate_kbps?: number;
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

/** 周期输入硬上限（秒换算 ms）；用户填 min–max，每周期在闭区间内随机。 */
export const PERIOD_HARD_MIN_MS = 1_000;
export const PERIOD_HARD_MAX_MS = 60_000;

// ponytail: 默认别太短，短周期会频繁换轨/跳 Tag；听感验收用 8–15s
export const DEFAULT_AUDIO_PERIOD_MIN_MS = 8_000;
export const DEFAULT_AUDIO_PERIOD_MAX_MS = 15_000;
export const DEFAULT_VIDEO_PERIOD_MIN_MS = 8_000;
export const DEFAULT_VIDEO_PERIOD_MAX_MS = 15_000;

/** @deprecated 兼容旧常量名 */
export const DEFAULT_AUDIO_VARIATION_PERIOD_MS = DEFAULT_AUDIO_PERIOD_MIN_MS;
export const MIN_AUDIO_VARIATION_PERIOD_MS = PERIOD_HARD_MIN_MS;
export const MAX_AUDIO_VARIATION_PERIOD_MS = PERIOD_HARD_MAX_MS;
export const DEFAULT_VIDEO_VARIATION_PERIOD_MS = DEFAULT_VIDEO_PERIOD_MIN_MS;
export const MIN_VIDEO_VARIATION_PERIOD_MS = PERIOD_HARD_MIN_MS;
export const MAX_VIDEO_VARIATION_PERIOD_MS = PERIOD_HARD_MAX_MS;

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

function normalizePeriodMs(
  periodMs: number | null | undefined,
  fallbackMs: number,
): number {
  if (!Number.isFinite(periodMs as number) || (periodMs as number) <= 0) {
    return fallbackMs;
  }
  return clamp(Math.round(periodMs as number), PERIOD_HARD_MIN_MS, PERIOD_HARD_MAX_MS);
}

export type PeriodRangeMs = { minMs: number; maxMs: number };

/** 归一化用户填写的 min–max；保证 hard 范围内且 min≤max。 */
export function normalizePeriodRangeMs(
  minMs: number | null | undefined,
  maxMs: number | null | undefined,
  defaults: PeriodRangeMs,
): PeriodRangeMs {
  let min = normalizePeriodMs(minMs, defaults.minMs);
  let max = normalizePeriodMs(maxMs, defaults.maxMs);
  if (min > max) {
    const swap = min;
    min = max;
    max = swap;
  }
  return { minMs: min, maxMs: max };
}

export function normalizeAudioPeriodRange(
  minMs?: number | null,
  maxMs?: number | null,
): PeriodRangeMs {
  return normalizePeriodRangeMs(minMs, maxMs, {
    minMs: DEFAULT_AUDIO_PERIOD_MIN_MS,
    maxMs: DEFAULT_AUDIO_PERIOD_MAX_MS,
  });
}

export function normalizeVideoPeriodRange(
  minMs?: number | null,
  maxMs?: number | null,
): PeriodRangeMs {
  return normalizePeriodRangeMs(minMs, maxMs, {
    minMs: DEFAULT_VIDEO_PERIOD_MIN_MS,
    maxMs: DEFAULT_VIDEO_PERIOD_MAX_MS,
  });
}

/** 在 [min,max] 闭区间随机取一整毫秒（含端点）。 */
export function samplePeriodMsInRange(
  range: PeriodRangeMs,
  random: () => number = Math.random,
): number {
  const { minMs, maxMs } = normalizePeriodRangeMs(range.minMs, range.maxMs, range);
  if (minMs === maxMs) return minMs;
  const span = maxMs - minMs;
  return minMs + Math.floor(random() * (span + 1));
}

/** @deprecated 兼容：把单值夹到硬范围 */
export function normalizeAudioVariationPeriod(periodMs?: number | null): number {
  return normalizePeriodMs(periodMs, DEFAULT_AUDIO_PERIOD_MIN_MS);
}

/** @deprecated */
export function normalizeVideoVariationPeriod(periodMs?: number | null): number {
  return normalizePeriodMs(periodMs, DEFAULT_VIDEO_PERIOD_MIN_MS);
}

/** @deprecated */
export function normalizeRuntimeVariationPeriod(periodMs?: number | null): number {
  return normalizeAudioVariationPeriod(periodMs);
}

export function isRuntimeVariationDue(
  nowMs: number,
  lastChangeMs: number,
  periodMs?: number | null,
): boolean {
  if (!Number.isFinite(nowMs) || !Number.isFinite(lastChangeMs)) {
    return false;
  }
  const effectivePeriodMs = normalizePeriodMs(periodMs, DEFAULT_AUDIO_PERIOD_MIN_MS);
  return nowMs - lastChangeMs >= effectivePeriodMs;
}

/** 一套完整声音参数值（约 20 项，不含采样率/码率/播放速度）。 */
export type SubtleAudioSample = {
  pitch_shift_semitones: number;
  input_gain_db: number;
  output_gain_db: number;
  loudness_adjustment_db: number;
  low_eq_db: number;
  mid_eq_db: number;
  high_eq_db: number;
  filter_q: number;
  phase_perturbation_percent: number;
  vibrato_frequency_hz: number;
  vibrato_depth_percent: number;
  reverb_wet_percent: number;
  noise_reduction_percent: number;
  environment_noise_percent: number;
  environment_noise_dbfs: number;
  fade_in_ms: number;
  fade_out_ms: number;
  dry_wet_percent: number;
  ambient_sound_mix_percent: number;
  spectral_perturbation_percent: number;
};

export type AudioValuePreset = {
  id: string;
  label: string;
  values: SubtleAudioSample;
};

function roundTo(value: number, digits: number): number {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

const AUDIO_PRESET_DEFAULTS: SubtleAudioSample = {
  pitch_shift_semitones: 0,
  input_gain_db: 0,
  output_gain_db: 0,
  loudness_adjustment_db: 0,
  low_eq_db: 0,
  mid_eq_db: 0,
  high_eq_db: 0,
  filter_q: 1,
  phase_perturbation_percent: 0,
  vibrato_frequency_hz: 5,
  vibrato_depth_percent: 0,
  reverb_wet_percent: 0,
  noise_reduction_percent: 0,
  environment_noise_percent: 0,
  environment_noise_dbfs: -48,
  fade_in_ms: 0,
  fade_out_ms: 0,
  dry_wet_percent: 0,
  ambient_sound_mix_percent: 0,
  spectral_perturbation_percent: 0,
};

function finiteClamp(value: number, fallback: number, minimum: number, maximum: number): number {
  return Number.isFinite(value) ? Math.min(maximum, Math.max(minimum, value)) : fallback;
}

/** 预设进入 IPC 前的安全边界；未映射字段固定为默认值。 */
export function sanitizeAudioPresetValues(values: SubtleAudioSample): SubtleAudioSample {
  return {
    ...values,
    pitch_shift_semitones: finiteClamp(values.pitch_shift_semitones, 0, -2, 2),
    input_gain_db: finiteClamp(values.input_gain_db, 0, -6, 6),
    output_gain_db: finiteClamp(values.output_gain_db, 0, -6, 6),
    loudness_adjustment_db: finiteClamp(values.loudness_adjustment_db, 0, -6, 6),
    low_eq_db: finiteClamp(values.low_eq_db, 0, -12, 12),
    mid_eq_db: finiteClamp(values.mid_eq_db, 0, -12, 12),
    high_eq_db: finiteClamp(values.high_eq_db, 0, -12, 12),
    filter_q: finiteClamp(values.filter_q, 1, 0.3, 10),
    phase_perturbation_percent: finiteClamp(values.phase_perturbation_percent, 0, -20, 20),
    vibrato_frequency_hz: finiteClamp(values.vibrato_frequency_hz, 5, 3, 8),
    vibrato_depth_percent: finiteClamp(values.vibrato_depth_percent, 0, 0, 3),
    reverb_wet_percent: finiteClamp(values.reverb_wet_percent, 0, 0, 20),
    noise_reduction_percent: finiteClamp(values.noise_reduction_percent, 0, 0, 100),
    environment_noise_percent: finiteClamp(values.environment_noise_percent, 0, 0, 100),
    environment_noise_dbfs: finiteClamp(values.environment_noise_dbfs, -48, -60, -20),
    fade_in_ms: Math.round(finiteClamp(values.fade_in_ms, 0, 0, 10_000)),
    fade_out_ms: Math.round(finiteClamp(values.fade_out_ms, 0, 0, 10_000)),
    dry_wet_percent: 0,
    ambient_sound_mix_percent: 0,
    spectral_perturbation_percent: 0,
  };
}

function audioValues(partial: Partial<SubtleAudioSample>): SubtleAudioSample {
  return sanitizeAudioPresetValues({
    ...AUDIO_PRESET_DEFAULTS,
    ...partial,
  });
}

/** 每个预设=完整 20 参数值；勾选多个后周期随机抽一套应用。 */
export const AUDIO_VALUE_PRESETS: readonly AudioValuePreset[] = [
  { id: 'p01', label: '1. 自然平直', values: audioValues({}) },
  { id: 'p02', label: '2. 微抬增益', values: audioValues({ input_gain_db: 0.18, output_gain_db: 0.12, loudness_adjustment_db: 0.15 }) },
  { id: 'p03', label: '3. 微降增益', values: audioValues({ input_gain_db: -0.16, output_gain_db: -0.1, loudness_adjustment_db: -0.12 }) },
  { id: 'p04', label: '4. 暖低频', values: audioValues({ low_eq_db: 0.45, mid_eq_db: -0.1, high_eq_db: -0.2, filter_q: 1.05 }) },
  { id: 'p05', label: '5. 亮高频', values: audioValues({ low_eq_db: -0.15, mid_eq_db: 0.1, high_eq_db: 0.5, filter_q: 1.08 }) },
  { id: 'p06', label: '6. 中频突出', values: audioValues({ mid_eq_db: 0.4, low_eq_db: -0.1, high_eq_db: -0.1, filter_q: 1.12 }) },
  { id: 'p07', label: '7. 轻上移音高', values: audioValues({ pitch_shift_semitones: 0.05, vibrato_depth_percent: 0.12, vibrato_frequency_hz: 5.1 }) },
  { id: 'p08', label: '8. 轻下移音高', values: audioValues({ pitch_shift_semitones: -0.05, vibrato_depth_percent: 0.1, vibrato_frequency_hz: 4.8 }) },
  { id: 'p09', label: '9. 轻颤音', values: audioValues({ vibrato_frequency_hz: 5.4, vibrato_depth_percent: 0.28, phase_perturbation_percent: 0.8 }) },
  // ponytail: 预设只写 FFmpeg 已映射字段；dry_wet/ambient_sound_mix/spectral 未映射，非 0 会拒渲染
  { id: 'p10', label: '10. 相位微扰', values: audioValues({ phase_perturbation_percent: 1.8 }) },
  { id: 'p11', label: '11. 轻混响', values: audioValues({ reverb_wet_percent: 1.1, fade_in_ms: 12, fade_out_ms: 16 }) },
  { id: 'p12', label: '12. 干声收紧', values: audioValues({ reverb_wet_percent: 0.2, noise_reduction_percent: 1.2 }) },
  { id: 'p13', label: '13. 轻降噪', values: audioValues({ noise_reduction_percent: 1.6, high_eq_db: -0.15 }) },
  { id: 'p14', label: '14. 底噪纹理', values: audioValues({ environment_noise_percent: 0.55, environment_noise_dbfs: -49 }) },
  { id: 'p15', label: '15. 淡入淡出', values: audioValues({ fade_in_ms: 22, fade_out_ms: 26, loudness_adjustment_db: -0.08 }) },
  { id: 'p16', label: '16. 自然微变', values: audioValues({ pitch_shift_semitones: 0.03, input_gain_db: 0.1, output_gain_db: 0.08, loudness_adjustment_db: 0.1, low_eq_db: 0.2, mid_eq_db: -0.05, high_eq_db: 0.15 }) },
  { id: 'p17', label: '17. 音色着色', values: audioValues({ low_eq_db: 0.25, mid_eq_db: 0.2, high_eq_db: 0.35, filter_q: 1.1, reverb_wet_percent: 0.7, vibrato_depth_percent: 0.15 }) },
  { id: 'p18', label: '18. 空间感', values: audioValues({ reverb_wet_percent: 1.3, phase_perturbation_percent: 1.2, fade_in_ms: 14, fade_out_ms: 18 }) },
  { id: 'p19', label: '19. 调制组合', values: audioValues({ vibrato_frequency_hz: 5.6, vibrato_depth_percent: 0.3, phase_perturbation_percent: 1.5 }) },
  { id: 'p20', label: '20. 综合微扰', values: audioValues({ pitch_shift_semitones: -0.02, input_gain_db: 0.12, output_gain_db: -0.05, loudness_adjustment_db: 0.08, low_eq_db: 0.18, mid_eq_db: -0.12, high_eq_db: 0.22, filter_q: 1.06, phase_perturbation_percent: 1.1, vibrato_frequency_hz: 5.2, vibrato_depth_percent: 0.18, reverb_wet_percent: 0.6, noise_reduction_percent: 0.8, environment_noise_percent: 0.25, environment_noise_dbfs: -50, fade_in_ms: 10, fade_out_ms: 12 }) },
  // ponytail: 听感验收用；直接顶契约上限，故意夸张
  { id: 'p21', label: '21. 明显加轨', values: audioValues({
    pitch_shift_semitones: 2.0,
    input_gain_db: 4.0,
    output_gain_db: 3.0,
    loudness_adjustment_db: 3.0,
    low_eq_db: 10.0,
    mid_eq_db: -8.0,
    high_eq_db: 10.0,
    filter_q: 6.0,
    phase_perturbation_percent: 20,
    vibrato_frequency_hz: 7.5,
    vibrato_depth_percent: 3.0,
    reverb_wet_percent: 20,
    environment_noise_percent: 55,
    environment_noise_dbfs: -22,
    fade_in_ms: 180,
    fade_out_ms: 220,
  }) },
  { id: 'p22', label: '22. 明显变调与空间（手动）', values: audioValues({
    pitch_shift_semitones: 2.0,
    low_eq_db: -6.0,
    mid_eq_db: 5.0,
    high_eq_db: 6.0,
    filter_q: 2.2,
    phase_perturbation_percent: 8.0,
    vibrato_frequency_hz: 6.8,
    vibrato_depth_percent: 2.5,
    reverb_wet_percent: 12.0,
  }) },
] as const;

// p21 仅保留为手动验收项；p22 可显式选择，但两者都不进入默认随机池。
const SELECTABLE_AUDIO_VALUE_PRESET_IDS: readonly string[] = AUDIO_VALUE_PRESETS
  .filter((preset) => preset.id !== 'p21')
  .map((preset) => preset.id);
export const DEFAULT_AUDIO_VALUE_PRESET_IDS: readonly string[] = AUDIO_VALUE_PRESETS
  .filter((preset) => preset.id !== 'p21' && preset.id !== 'p22')
  .map((preset) => preset.id);

export const AUDIO_MIX_PICK_HARD_MAX = 4;
export const DEFAULT_AUDIO_MIX_PICK_MIN = 1;
export const DEFAULT_AUDIO_MIX_PICK_MAX = 2;

export function normalizeAudioMixPickMax(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_AUDIO_MIX_PICK_MAX;
  return Math.min(AUDIO_MIX_PICK_HARD_MAX, Math.max(1, Math.round(value)));
}

export function normalizeAudioMixPickMin(value: number, pickMax: number): number {
  const max = normalizeAudioMixPickMax(pickMax);
  if (!Number.isFinite(value)) return Math.min(DEFAULT_AUDIO_MIX_PICK_MIN, max);
  return Math.min(max, Math.max(1, Math.round(value)));
}

export function getAudioValuePreset(id: string): AudioValuePreset {
  return AUDIO_VALUE_PRESETS.find((preset) => preset.id === id) ?? AUDIO_VALUE_PRESETS[0];
}

/** 从不放回抽样中选出 k 个预设 ID。 */
export function pickAudioPresetIds(
  selectedPresetIds: readonly string[],
  pickMin: number,
  pickMax: number,
  random = Math.random,
  previousPresetIds: readonly string[] = [],
): string[] {
  const pool = [...new Set(selectedPresetIds)].filter((id) =>
    SELECTABLE_AUDIO_VALUE_PRESET_IDS.includes(id),
  );
  if (pool.length === 0) return [];
  // 候选超过 1 套时禁止与上一周期重叠；这样 A → B → A 可以，A → A 不可以。
  // 如果本轮要求的轨数超过剩余候选数，缩小本轮轨数，不重新引入上一周期的预设。
  const previous = new Set(previousPresetIds);
  const nonRepeatingPool = pool.length > 1
    ? pool.filter((id) => !previous.has(id))
    : pool;
  const effectivePool = nonRepeatingPool.length > 0 ? nonRepeatingPool : pool;
  const max = Math.min(normalizeAudioMixPickMax(pickMax), effectivePool.length);
  const min = Math.min(normalizeAudioMixPickMin(pickMin, max), max);
  const count = min + Math.floor(random() * (max - min + 1));
  const shuffled = [...effectivePool];
  for (let index = shuffled.length - 1; index > 0; index -= 1) {
    const swapIndex = Math.floor(random() * (index + 1));
    const current = shuffled[index];
    shuffled[index] = shuffled[swapIndex];
    shuffled[swapIndex] = current;
  }
  return shuffled.slice(0, count);
}

export type AudioCycleSample = {
  presetIds: string[];
  /** 第一套，供面板展示 / 单轨 params.audio */
  values: SubtleAudioSample;
  /** 本周期全部虚拟轨参数（等权 amix） */
  variants: SubtleAudioSample[];
  weights: number[];
  seed: number;
};

export function equalMixWeights(count: number): number[] {
  if (count <= 0) return [];
  const weight = 1 / count;
  return Array.from({ length: count }, () => weight);
}

/** 播放端参数切换交叉淡化时长（秒）；计划 20–40ms。 */
export const AUDIO_PARAM_CROSSFADE_SEC = 0.03;

export function buildAudioFxSignature(
  params: {
    audio_input_gain_db?: number;
    audio_output_gain_db?: number;
    audio_loudness_adjustment_db?: number;
    audio_gain_db?: number;
    audio_filter_q?: number;
    audio_low_eq_db?: number;
    audio_mid_eq_db?: number;
    audio_high_eq_db?: number;
    audio_pitch_shift_semitones?: number;
    audio_reverb_wet_percent?: number;
    audio_environment_noise_percent?: number;
    audio_environment_noise_dbfs?: number;
  } | null,
  audioEnabled: boolean,
): string {
  if (!audioEnabled || !params) return 'off';
  const totalDb =
    (params.audio_input_gain_db ?? 0)
    + (params.audio_output_gain_db ?? 0)
    + (params.audio_loudness_adjustment_db ?? params.audio_gain_db ?? 0);
  return [
    totalDb.toFixed(4),
    Math.max(0.3, Math.min(10, params.audio_filter_q ?? 1)).toFixed(4),
    (params.audio_low_eq_db ?? 0).toFixed(4),
    (params.audio_mid_eq_db ?? 0).toFixed(4),
    (params.audio_high_eq_db ?? 0).toFixed(4),
    (params.audio_pitch_shift_semitones ?? 0).toFixed(4),
    (params.audio_reverb_wet_percent ?? 0).toFixed(4),
    (params.audio_environment_noise_percent ?? 0).toFixed(4),
    (params.audio_environment_noise_dbfs ?? -40).toFixed(4),
  ].join('|');
}

/** 可复现伪随机；测试可直接注入 random 跳过。 */
function mulberry32(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let next = Math.imul(state ^ (state >>> 15), 1 | state);
    next ^= next + Math.imul(next ^ (next >>> 7), 61 | next);
    return ((next ^ (next >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * 周期抽样。
 * mixEnabled=false 或 max=1：只抽 1 套。
 * mixEnabled=true：抽 [min,max] 套，variants 供 FFmpeg asplit/amix。
 * previousPresetIds：上一周期参与的预设；候选超过 1 套时禁止相邻周期重复。
 */
export function sampleAudioCycle(
  selectedPresetIds: readonly string[] = DEFAULT_AUDIO_VALUE_PRESET_IDS,
  options: {
    mixEnabled?: boolean;
    pickMin?: number;
    pickMax?: number;
    seed?: number;
    random?: () => number;
    previousPresetIds?: readonly string[];
  } = {},
): AudioCycleSample {
  const seed =
    Number.isFinite(options.seed) ? Math.floor(options.seed as number) >>> 0 : Math.floor(Math.random() * 0x7fffffff);
  const random = options.random ?? mulberry32(seed);
  const mixEnabled = Boolean(options.mixEnabled);
  const pickMax = mixEnabled ? normalizeAudioMixPickMax(options.pickMax ?? DEFAULT_AUDIO_MIX_PICK_MAX) : 1;
  const pickMin = mixEnabled
    ? normalizeAudioMixPickMin(options.pickMin ?? DEFAULT_AUDIO_MIX_PICK_MIN, pickMax)
    : 1;
  const presetIds = pickAudioPresetIds(
    selectedPresetIds,
    pickMin,
    pickMax,
    random,
    options.previousPresetIds,
  );
  if (presetIds.length === 0) {
    return { presetIds: [], values: audioValues({}), variants: [], weights: [], seed };
  }
  // 保留预设原值；未映射字段由 Rust 边界明确拒绝，不能在这里静默改写。
  const variants = presetIds.map((id) => ({ ...getAudioValuePreset(id).values }));
  return {
    presetIds,
    values: { ...variants[0] },
    variants,
    weights: equalMixWeights(variants.length),
    seed,
  };
}

export const AUDIO_PERIOD_RANGE_STORAGE_KEY = 'autolive.audio-period-range-ms.v1';
export const VIDEO_PERIOD_RANGE_STORAGE_KEY = 'autolive.video-period-range-ms.v1';
/** @deprecated */
export const VIDEO_VARIATION_PERIOD_STORAGE_KEY = VIDEO_PERIOD_RANGE_STORAGE_KEY;

function loadPeriodRange(
  key: string,
  defaults: PeriodRangeMs,
  storage?: Pick<Storage, 'getItem'> | null,
): PeriodRangeMs {
  const target =
    storage ?? (typeof window !== 'undefined' && window.localStorage ? window.localStorage : null);
  if (!target) return defaults;
  try {
    const raw = target.getItem(key);
    if (!raw) return defaults;
    // 兼容旧单值：纯数字 → min=max
    if (/^\d+$/.test(raw.trim())) {
      const one = normalizePeriodMs(Number(raw), defaults.minMs);
      return normalizePeriodRangeMs(one, one, defaults);
    }
    const parsed = JSON.parse(raw) as { minMs?: number; maxMs?: number };
    return normalizePeriodRangeMs(parsed.minMs, parsed.maxMs, defaults);
  } catch {
    return defaults;
  }
}

function savePeriodRange(
  key: string,
  range: PeriodRangeMs,
  defaults: PeriodRangeMs,
  storage?: Pick<Storage, 'setItem'> | null,
): void {
  const target =
    storage ?? (typeof window !== 'undefined' && window.localStorage ? window.localStorage : null);
  if (!target) return;
  const normalized = normalizePeriodRangeMs(range.minMs, range.maxMs, defaults);
  target.setItem(key, JSON.stringify(normalized));
}

export function loadAudioPeriodRange(storage?: Pick<Storage, 'getItem'> | null): PeriodRangeMs {
  return loadPeriodRange(
    AUDIO_PERIOD_RANGE_STORAGE_KEY,
    { minMs: DEFAULT_AUDIO_PERIOD_MIN_MS, maxMs: DEFAULT_AUDIO_PERIOD_MAX_MS },
    storage,
  );
}

export function saveAudioPeriodRange(
  range: PeriodRangeMs,
  storage?: Pick<Storage, 'setItem'> | null,
): void {
  savePeriodRange(
    AUDIO_PERIOD_RANGE_STORAGE_KEY,
    range,
    { minMs: DEFAULT_AUDIO_PERIOD_MIN_MS, maxMs: DEFAULT_AUDIO_PERIOD_MAX_MS },
    storage,
  );
}

export function loadVideoPeriodRange(storage?: Pick<Storage, 'getItem'> | null): PeriodRangeMs {
  return loadPeriodRange(
    VIDEO_PERIOD_RANGE_STORAGE_KEY,
    { minMs: DEFAULT_VIDEO_PERIOD_MIN_MS, maxMs: DEFAULT_VIDEO_PERIOD_MAX_MS },
    storage,
  );
}

export function saveVideoPeriodRange(
  range: PeriodRangeMs,
  storage?: Pick<Storage, 'setItem'> | null,
): void {
  savePeriodRange(
    VIDEO_PERIOD_RANGE_STORAGE_KEY,
    range,
    { minMs: DEFAULT_VIDEO_PERIOD_MIN_MS, maxMs: DEFAULT_VIDEO_PERIOD_MAX_MS },
    storage,
  );
}

/** @deprecated */
export function loadVideoVariationPeriodMs(storage?: Pick<Storage, 'getItem'> | null): number {
  return loadVideoPeriodRange(storage).minMs;
}

/** @deprecated */
export function saveVideoVariationPeriodMs(
  periodMs: number,
  storage?: Pick<Storage, 'setItem'> | null,
): void {
  const one = normalizeVideoVariationPeriod(periodMs);
  saveVideoPeriodRange({ minMs: one, maxMs: one }, storage);
}

export const AUDIO_MIX_SESSION_STORAGE_KEY = 'autolive.audio-mix-session.v1';

export type AudioMixSession = {
  selectedPresetIds: string[];
  mixEnabled: boolean;
  pickMin: number;
  pickMax: number;
};

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

export function loadAudioMixSession(storage?: StorageLike | null): AudioMixSession | null {
  const target =
    storage ?? (typeof window !== 'undefined' && window.localStorage ? window.localStorage : null);
  if (!target) return null;
  try {
    const raw = target.getItem(AUDIO_MIX_SESSION_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<AudioMixSession>;
    if (!Array.isArray(parsed.selectedPresetIds)) return null;
    const selectedPresetIds = [...new Set(parsed.selectedPresetIds.map(String))].filter((id) =>
      SELECTABLE_AUDIO_VALUE_PRESET_IDS.includes(id),
    );
    const pickMax = normalizeAudioMixPickMax(Number(parsed.pickMax));
    const pickMin = normalizeAudioMixPickMin(Number(parsed.pickMin), pickMax);
    return {
      selectedPresetIds,
      mixEnabled: Boolean(parsed.mixEnabled),
      pickMin,
      pickMax,
    };
  } catch {
    return null;
  }
}

export function saveAudioMixSession(session: AudioMixSession, storage?: StorageLike | null): void {
  const target =
    storage ?? (typeof window !== 'undefined' && window.localStorage ? window.localStorage : null);
  if (!target) return;
  const pickMax = normalizeAudioMixPickMax(session.pickMax);
  const pickMin = normalizeAudioMixPickMin(session.pickMin, pickMax);
  const selectedPresetIds = [...new Set(session.selectedPresetIds.map(String))].filter((id) =>
    SELECTABLE_AUDIO_VALUE_PRESET_IDS.includes(id),
  );
  target.setItem(
    AUDIO_MIX_SESSION_STORAGE_KEY,
    JSON.stringify({
      selectedPresetIds,
      mixEnabled: Boolean(session.mixEnabled),
      pickMin,
      pickMax,
    } satisfies AudioMixSession),
  );
}

/** FFmpeg 尚未映射的声音字段；非默认值必须显式提示并由 Rust 边界拒绝。 */
export const UNMAPPED_AUDIO_PRESET_FIELDS = [
  'dry_wet_percent',
  'ambient_sound_mix_percent',
  'spectral_perturbation_percent',
] as const;

export type UnmappedAudioPresetField = (typeof UNMAPPED_AUDIO_PRESET_FIELDS)[number];

export const UNMAPPED_AUDIO_PRESET_FIELD_LABELS: Record<UnmappedAudioPresetField, string> = {
  dry_wet_percent: '干湿比',
  ambient_sound_mix_percent: '环境声混合',
  spectral_perturbation_percent: '频谱微扰',
};

export function getUnsupportedAudioPresetFields(
  values: Partial<Pick<SubtleAudioSample, UnmappedAudioPresetField>>,
): UnmappedAudioPresetField[] {
  return UNMAPPED_AUDIO_PRESET_FIELDS.filter((field) => {
    const value = values[field];
    return value !== undefined && (!Number.isFinite(value) || Math.abs(value) > Number.EPSILON);
  });
}

/** 把虚拟轨微扰叠到完整 audio 参数上；采样率/码率/播放速度/周期等跟主轨。 */
export function buildAudioVariantsFromCycle<T extends Record<string, unknown>>(
  baseAudio: T,
  sample: Pick<AudioCycleSample, 'variants'>,
): T[] {
  return sample.variants.map((variant) => ({
    ...baseAudio,
    ...variant,
  }));
}

/** @deprecated 兼容旧调用：等价于只抽 1 套 */
export function sampleSubtleAudioParams(
  selectedPresetIds: readonly string[] = DEFAULT_AUDIO_VALUE_PRESET_IDS,
  random = Math.random,
): SubtleAudioSample {
  return sampleAudioCycle(selectedPresetIds, { mixEnabled: false, random }).values;
}

/** 与音频同周期的视频研究参数微扰；写入 researchParams.video 后实时预览/应用共用。 */
export type SubtleVideoSample = {
  brightness_percent: number;
  contrast_percent: number;
  saturation_percent: number;
  hue_rotation_degrees: number;
  blur_radius_px: number;
  pixel_scale_percent: number;
  space_x_offset_px: number;
  space_y_offset_px: number;
  sharpen_percent: number;
  noise_percent: number;
  detail_enhancement_percent: number;
  dynamic_crop_percent: number;
  pixel_jitter_px: number;
  crop_edge_smoothing: number;
  frame_rate_jitter_percent: number;
  frame_rate_perturbation_frequency_hz: number;
  frame_rate_perturbation_amplitude_fps: number;
  frame_inner_perturbation_percent: number;
  frame_inter_perturbation_percent: number;
  color_space_conversion_strength_percent: number;
};

export type VideoCycleSample = {
  seed: number;
  values: SubtleVideoSample;
};

export function sampleVideoCycle(seed = Math.floor(Math.random() * 0x7fffffff)): VideoCycleSample {
  const normalizedSeed = Number.isFinite(seed) ? Math.floor(seed) >>> 0 : 0;
  return {
    seed: normalizedSeed,
    values: sampleSubtleVideoParams(mulberry32(normalizedSeed)),
  };
}

/** FFmpeg 已映射的视频微扰；未映射字段保持契约默认，避免 Worker 拒渲染。 */
export function sampleSubtleVideoParams(random = Math.random): SubtleVideoSample {
  const inRange = (min: number, max: number) => min + random() * (max - min);
  return sanitizeMappedVideoSample({
    brightness_percent: roundTo(inRange(-3, 3), 2),
    contrast_percent: roundTo(inRange(97, 103), 2),
    saturation_percent: roundTo(inRange(97, 103), 2),
    hue_rotation_degrees: roundTo(inRange(-3, 3), 2),
    blur_radius_px: roundTo(inRange(0, 0.4), 2),
    pixel_scale_percent: roundTo(inRange(99.5, 100.5), 2),
    space_x_offset_px: roundTo(inRange(-0.5, 0.5), 2),
    space_y_offset_px: roundTo(inRange(-0.5, 0.5), 2),
    sharpen_percent: roundTo(inRange(0, 3), 2),
    noise_percent: roundTo(inRange(0, 0.5), 2),
    detail_enhancement_percent: roundTo(inRange(0, 2), 2),
    dynamic_crop_percent: roundTo(inRange(0, 0.3), 2),
    pixel_jitter_px: roundTo(inRange(0, 0.2), 2),
    // 以下字段 Worker 未映射：必须保持默认
    crop_edge_smoothing: 0.5,
    frame_rate_jitter_percent: 0,
    frame_rate_perturbation_frequency_hz: 0.1,
    frame_rate_perturbation_amplitude_fps: 0,
    frame_inner_perturbation_percent: 0,
    frame_inter_perturbation_percent: 0,
    color_space_conversion_strength_percent: 0,
  });
}

export function sanitizeMappedVideoSample(values: SubtleVideoSample): SubtleVideoSample {
  return {
    ...values,
    crop_edge_smoothing: 0.5,
    frame_rate_jitter_percent: 0,
    frame_rate_perturbation_frequency_hz: 0.1,
    frame_rate_perturbation_amplitude_fps: 0,
    frame_inner_perturbation_percent: 0,
    frame_inter_perturbation_percent: 0,
    color_space_conversion_strength_percent: 0,
  };
}

const VISUAL_BANDS_HZ = [65, 92, 131, 188, 267, 381, 544, 777, 1110, 1585, 2263, 20_000] as const;

/** 视觉调制/挂件/切片微扰；写入 researchParams.research。 */
export type SubtleResearchSample = {
  band_weights: Record<string, number>;
  target_frequency_hz: number | null;
  core_frequency_hz: number | null;
  wave_intensity: number;
  wave_level: number;
  wave_grain_count: number;
  dynamic_eq_threshold: number;
  channel_offset_percent: number;
  space_dimension: number;
  frequency_space_x_offset_px: number;
  frequency_space_y_offset_px: number;
  frame_perturbation_probability_percent: number;
  random_graphic_opacity_percent: number;
  random_graphic_size_px: number;
  abstract_face_count: number;
  abstract_face_size_percent: number;
  abstract_face_opacity_percent: number;
  overlay_offset_px: number;
  slice_length_ms: number;
  slice_min_length_ms: number;
  slice_trigger_interval_ms: number;
};

export function sampleSubtleResearchParams(random = Math.random): SubtleResearchSample {
  const inRange = (min: number, max: number) => min + random() * (max - min);
  const band_weights: Record<string, number> = {};
  for (const hz of VISUAL_BANDS_HZ) {
    band_weights[String(hz)] = roundTo(inRange(0.95, 1.05), 3);
  }
  const target = roundTo(inRange(65, 200), 1);
  const sliceLength = Math.round(inRange(500, 5_000));
  const sliceTrigger = Math.max(sliceLength, Math.round(inRange(5_000, 30_000)));
  return {
    band_weights,
    target_frequency_hz: target,
    core_frequency_hz: target,
    wave_intensity: roundTo(inRange(0, 0.15), 3),
    wave_level: roundTo(inRange(0, 0.15), 3),
    wave_grain_count: Math.round(inRange(10, 40)),
    dynamic_eq_threshold: roundTo(inRange(8, 12), 2),
    channel_offset_percent: roundTo(inRange(-1, 1), 2),
    space_dimension: 2,
    frequency_space_x_offset_px: roundTo(inRange(-1, 1), 2),
    frequency_space_y_offset_px: roundTo(inRange(-1, 1), 2),
    frame_perturbation_probability_percent: roundTo(inRange(0, 2), 2),
    random_graphic_opacity_percent: roundTo(inRange(0, 5), 2),
    random_graphic_size_px: roundTo(inRange(2, 8), 1),
    abstract_face_count: Math.round(inRange(0, 2)),
    abstract_face_size_percent: roundTo(inRange(1, 3), 2),
    abstract_face_opacity_percent: roundTo(inRange(0, 5), 2),
    overlay_offset_px: roundTo(inRange(-1, 1), 2),
    slice_length_ms: sliceLength,
    slice_min_length_ms: Math.max(1_000, Math.round(inRange(1_000, 15_000))),
    slice_trigger_interval_ms: sliceTrigger,
  };
}

export function buildRuntimePreviewParameters(
  baseParameters: RuntimeBaseParameters,
  cycle: number,
): RuntimePreviewParameters {
  const safeCycle = Number.isFinite(cycle) ? cycle : 0;

  return {
    // 音频参数只作为 FFmpeg 处理请求的快照传递；周期调度只改变视频预览，
    // 避免在播放器端用 Web Audio 伪造“实时声音效果”。
    audio_gain_db: clamp(finiteOrZero(baseParameters.audio_gain_db), -6, 6),
    audio_low_eq_db: clamp(finiteOrZero(baseParameters.audio_low_eq_db), -12, 12),
    audio_mid_eq_db: clamp(finiteOrZero(baseParameters.audio_mid_eq_db), -12, 12),
    audio_high_eq_db: clamp(finiteOrZero(baseParameters.audio_high_eq_db), -12, 12),
    audio_input_gain_db: baseParameters.audio_input_gain_db,
    audio_output_gain_db: baseParameters.audio_output_gain_db,
    audio_loudness_adjustment_db: baseParameters.audio_loudness_adjustment_db,
    audio_pitch_shift_semitones: baseParameters.audio_pitch_shift_semitones,
    audio_playback_speed: baseParameters.audio_playback_speed,
    audio_fade_in_ms: baseParameters.audio_fade_in_ms,
    audio_fade_out_ms: baseParameters.audio_fade_out_ms,
    audio_reverb_wet_percent: baseParameters.audio_reverb_wet_percent,
    audio_noise_reduction_percent: baseParameters.audio_noise_reduction_percent,
    audio_phase_perturbation_percent: baseParameters.audio_phase_perturbation_percent,
    audio_vibrato_frequency_hz: baseParameters.audio_vibrato_frequency_hz,
    audio_vibrato_depth_percent: baseParameters.audio_vibrato_depth_percent,
    audio_environment_noise_percent: baseParameters.audio_environment_noise_percent,
    audio_environment_noise_dbfs: baseParameters.audio_environment_noise_dbfs,
    audio_filter_q: baseParameters.audio_filter_q,
    audio_sample_rate_hz: baseParameters.audio_sample_rate_hz,
    audio_output_bitrate_kbps: baseParameters.audio_output_bitrate_kbps,
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
