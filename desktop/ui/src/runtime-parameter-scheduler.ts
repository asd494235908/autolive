import {
  DEFAULT_AUDIO_VALUE_PRESET_IDS,
  getAudioValuePreset,
  isSelectableAudioValuePresetId,
} from './audio-value-presets';
import type { SubtleAudioSample } from './audio-value-presets';
import type { MediaEffectParams } from './media-parameter-panels/media-parameter-types';

export {
  AUDIO_PRESET_FIELDS,
  AUDIO_VALUE_PRESETS,
  DEFAULT_AUDIO_VALUE_PRESET_IDS,
  getAudioValuePreset,
  sanitizeAudioPresetValues,
} from './audio-value-presets';
export type { AudioValuePreset, SubtleAudioSample } from './audio-value-presets';

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
  video_rotation_degrees: number;
  video_horizontal_flip_enabled: boolean;
  video_vertical_flip_enabled: boolean;
};

export type RuntimePreviewParameters = RuntimeBaseParameters;

export function toRuntimePreviewParameters(
  params: Pick<MediaEffectParams, 'audio' | 'video'>,
): RuntimePreviewParameters {
  return {
    audio_gain_db:
      params.audio.input_gain_db
      + params.audio.output_gain_db
      + params.audio.loudness_adjustment_db,
    audio_low_eq_db: params.audio.low_eq_db,
    audio_mid_eq_db: params.audio.mid_eq_db,
    audio_high_eq_db: params.audio.high_eq_db,
    audio_input_gain_db: params.audio.input_gain_db,
    audio_output_gain_db: params.audio.output_gain_db,
    audio_loudness_adjustment_db: params.audio.loudness_adjustment_db,
    audio_pitch_shift_semitones: params.audio.pitch_shift_semitones,
    audio_playback_speed: params.audio.playback_speed,
    audio_fade_in_ms: params.audio.fade_in_ms,
    audio_fade_out_ms: params.audio.fade_out_ms,
    audio_reverb_wet_percent: params.audio.reverb_wet_percent,
    audio_noise_reduction_percent: params.audio.noise_reduction_percent,
    audio_phase_perturbation_percent: params.audio.phase_perturbation_percent,
    audio_vibrato_frequency_hz: params.audio.vibrato_frequency_hz,
    audio_vibrato_depth_percent: params.audio.vibrato_depth_percent,
    audio_environment_noise_percent: params.audio.environment_noise_percent,
    audio_environment_noise_dbfs: params.audio.environment_noise_dbfs,
    audio_filter_q: params.audio.filter_q,
    audio_sample_rate_hz: params.audio.sample_rate_hz ?? 0,
    audio_output_bitrate_kbps: params.audio.output_bitrate_kbps,
    video_brightness_percent: params.video.brightness_percent,
    video_contrast_percent: params.video.contrast_percent,
    video_saturation_percent: params.video.saturation_percent,
    video_hue_rotation_degrees: params.video.hue_rotation_degrees,
    video_blur_radius_px: params.video.blur_radius_px,
    video_pixel_scale_percent: params.video.pixel_scale_percent,
    video_space_x_offset_px: params.video.space_x_offset_px,
    video_space_y_offset_px: params.video.space_y_offset_px,
    video_rotation_degrees: params.video.rotation_degrees,
    video_horizontal_flip_enabled: params.video.horizontal_flip_enabled,
    video_vertical_flip_enabled: params.video.vertical_flip_enabled,
  };
}

/** 周期输入硬上限（秒换算 ms）；用户填 min–max，每周期在闭区间内随机。 */
export const PERIOD_HARD_MIN_MS = 1_000;
export const PERIOD_HARD_MAX_MS = 60_000;

// 普通声音新配置与恢复默认使用 3–5s；已有本地保存值继续按原值读取。
export const DEFAULT_AUDIO_PERIOD_MIN_MS = 3_000;
export const DEFAULT_AUDIO_PERIOD_MAX_MS = 5_000;
// 视频新配置默认使用 5–8s；已有本地保存值继续按原值读取。
export const DEFAULT_VIDEO_PERIOD_MIN_MS = 5_000;
export const DEFAULT_VIDEO_PERIOD_MAX_MS = 8_000;

/** @deprecated 兼容旧常量名 */
export const DEFAULT_AUDIO_VARIATION_PERIOD_MS = DEFAULT_AUDIO_PERIOD_MIN_MS;
export const MIN_AUDIO_VARIATION_PERIOD_MS = PERIOD_HARD_MIN_MS;
export const MAX_AUDIO_VARIATION_PERIOD_MS = PERIOD_HARD_MAX_MS;
export const DEFAULT_VIDEO_VARIATION_PERIOD_MS = DEFAULT_VIDEO_PERIOD_MIN_MS;
export const MIN_VIDEO_VARIATION_PERIOD_MS = PERIOD_HARD_MIN_MS;
export const MAX_VIDEO_VARIATION_PERIOD_MS = PERIOD_HARD_MAX_MS;

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
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
export type PeriodRangeEndpoint = 'min' | 'max';

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

/** 编辑单个端点时固定另一端；越界只收敛当前端点，不交换两端。 */
export function updatePeriodRangeEndpoint(
  current: PeriodRangeMs,
  endpoint: PeriodRangeEndpoint,
  valueMs: number,
): PeriodRangeMs {
  const next = normalizePeriodMs(valueMs, endpoint === 'min' ? current.minMs : current.maxMs);
  return endpoint === 'min'
    ? { minMs: Math.min(next, current.maxMs), maxMs: current.maxMs }
    : { minMs: current.minMs, maxMs: Math.max(next, current.minMs) };
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

function roundTo(value: number, digits: number): number {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

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

/** 从不放回抽样中选出 k 个预设 ID。 */
export function pickAudioPresetIds(
  selectedPresetIds: readonly string[],
  pickMin: number,
  pickMax: number,
  random = Math.random,
  previousPresetIds: readonly string[] = [],
): string[] {
  const pool = [...new Set(selectedPresetIds)].filter(isSelectableAudioValuePresetId);
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
    return {
      presetIds: [],
      values: { ...getAudioValuePreset('p01').values },
      variants: [],
      weights: [],
      seed,
    };
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
      isSelectableAudioValuePresetId(id),
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
    isSelectableAudioValuePresetId(id),
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

/** 把虚拟轨预设叠到完整 audio 参数上；共同时间轴和混音后总线字段由主预设统一。 */
export function buildAudioVariantsFromCycle<T extends Record<string, unknown>>(
  baseAudio: T,
  sample: Pick<AudioCycleSample, 'variants'>,
): T[] {
  return sample.variants.map((variant) => ({
    ...baseAudio,
    ...variant,
    playback_speed: baseAudio.playback_speed,
    sample_rate_hz: baseAudio.sample_rate_hz,
    output_bitrate_kbps: baseAudio.output_bitrate_kbps,
    pitch_shift_semitones: baseAudio.pitch_shift_semitones,
    formant_shift_percent: baseAudio.formant_shift_percent,
    mfcc_shift_percent: baseAudio.mfcc_shift_percent,
    mfcc_dimensions: baseAudio.mfcc_dimensions,
    snr_target_db: baseAudio.snr_target_db,
    snr_variation_db: baseAudio.snr_variation_db,
    ambient_sound_mix_percent: baseAudio.ambient_sound_mix_percent,
    current_formant_hz: null,
  }));
}

/** @deprecated 兼容旧调用：等价于只抽 1 套 */
export function sampleSubtleAudioParams(
  selectedPresetIds: readonly string[] = DEFAULT_AUDIO_VALUE_PRESET_IDS,
  random = Math.random,
): SubtleAudioSample {
  return sampleAudioCycle(selectedPresetIds, { mixEnabled: false, random }).values;
}
