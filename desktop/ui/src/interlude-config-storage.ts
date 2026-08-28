import { INTERLUDE_LIMITS, INTERLUDE_PRESET_PERIOD_LIMITS } from './interlude-player';

export const INTERLUDE_CONFIG_STORAGE_KEY = 'autolive.interlude-config.v1';

export type PersistedInterludeConfig = {
  enabled: boolean;
  directory: string | null;
  audio_selection_mode: 'fixed' | 'random';
  audio_fixed_preset_id: string;
  audio_preset_ids: string[];
  audio_mix_enabled: boolean;
  audio_mix_pick_min: number;
  audio_mix_pick_max: number;
  audio_variation_mode: 'each_playback' | 'periodic';
  audio_variation_period_min_ms: number;
  audio_variation_period_max_ms: number;
  interval_min_ms: number;
  interval_max_ms: number;
  volume_db: number;
  ducking_depth_db: number;
  ducking_attack_ms: number;
  ducking_release_ms: number;
};

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

function resolveStorage(storage?: StorageLike | null): StorageLike | null {
  if (storage !== undefined) return storage;
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
  } catch {
    return null;
  }
}

function isIntegerInRange(value: unknown, min: number, max: number): value is number {
  return Number.isSafeInteger(value) && Number(value) >= min && Number(value) <= max;
}

function isNumberInRange(value: unknown, min: number, max: number): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= min && value <= max;
}

function isPresetId(value: unknown): value is string {
  return typeof value === 'string' && /^p(?:0[1-9]|1\d|2[0-2])$/.test(value);
}

function isPersistedInterludeConfig(value: unknown): value is PersistedInterludeConfig {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const presetIds = record.audio_preset_ids;
  return typeof record.enabled === 'boolean'
    && (record.directory === null || (
      typeof record.directory === 'string'
      && record.directory.length > 0
      && record.directory.length <= 4_096
    ))
    && (record.audio_selection_mode === 'fixed' || record.audio_selection_mode === 'random')
    && isPresetId(record.audio_fixed_preset_id)
    && Array.isArray(presetIds)
    && presetIds.length >= 1
    && presetIds.length <= 22
    && presetIds.every(isPresetId)
    && new Set(presetIds).size === presetIds.length
    && typeof record.audio_mix_enabled === 'boolean'
    && isIntegerInRange(record.audio_mix_pick_min, 1, 4)
    && isIntegerInRange(record.audio_mix_pick_max, 1, 4)
    && record.audio_mix_pick_min <= record.audio_mix_pick_max
    && (record.audio_variation_mode === 'each_playback' || record.audio_variation_mode === 'periodic')
    && isIntegerInRange(
      record.audio_variation_period_min_ms,
      INTERLUDE_PRESET_PERIOD_LIMITS.min,
      INTERLUDE_PRESET_PERIOD_LIMITS.max,
    )
    && isIntegerInRange(
      record.audio_variation_period_max_ms,
      INTERLUDE_PRESET_PERIOD_LIMITS.min,
      INTERLUDE_PRESET_PERIOD_LIMITS.max,
    )
    && record.audio_variation_period_min_ms <= record.audio_variation_period_max_ms
    && isIntegerInRange(
      record.interval_min_ms,
      INTERLUDE_LIMITS.intervalMinMs.min,
      INTERLUDE_LIMITS.intervalMinMs.max,
    )
    && isIntegerInRange(
      record.interval_max_ms,
      INTERLUDE_LIMITS.intervalMinMs.min,
      INTERLUDE_LIMITS.intervalMinMs.max,
    )
    && record.interval_min_ms <= record.interval_max_ms
    && isNumberInRange(record.volume_db, INTERLUDE_LIMITS.volumeDb.min, INTERLUDE_LIMITS.volumeDb.max)
    && isNumberInRange(
      record.ducking_depth_db,
      INTERLUDE_LIMITS.duckingDepthDb.min,
      INTERLUDE_LIMITS.duckingDepthDb.max,
    )
    && isIntegerInRange(
      record.ducking_attack_ms,
      INTERLUDE_LIMITS.duckingAttackMs.min,
      INTERLUDE_LIMITS.duckingAttackMs.max,
    )
    && isIntegerInRange(
      record.ducking_release_ms,
      INTERLUDE_LIMITS.duckingReleaseMs.min,
      INTERLUDE_LIMITS.duckingReleaseMs.max,
    );
}

export function loadInterludeConfig(storage?: StorageLike | null): PersistedInterludeConfig | null {
  const target = resolveStorage(storage);
  if (!target) return null;
  try {
    const raw = target.getItem(INTERLUDE_CONFIG_STORAGE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    return isPersistedInterludeConfig(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

export function saveInterludeConfig(
  config: PersistedInterludeConfig,
  storage?: StorageLike | null,
): void {
  if (!isPersistedInterludeConfig(config)) throw new Error('随机插话配置内容无效');
  const target = resolveStorage(storage);
  if (!target) throw new Error('随机插话配置本地保存不可用');
  target.setItem(INTERLUDE_CONFIG_STORAGE_KEY, JSON.stringify(config));
}
