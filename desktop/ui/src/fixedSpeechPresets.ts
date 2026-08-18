export const FIXED_SPEECH_PRESETS_STORAGE_KEY = 'autolive.fixed-speech-presets.v1';
const LEGACY_PRESETS_STORAGE_KEY = 'autolive.voice-clone-presets.v1';
const MAX_PRESET_COUNT = 10;
const MAX_TITLE_LENGTH = 80;
const MAX_TEXT_LENGTH = 500;

export type FixedSpeechPreset = {
  id: string;
  title: string;
  text: string;
  createdAt: string;
  updatedAt: string;
};

type FixedSpeechPresetDraft = Pick<FixedSpeechPreset, 'title' | 'text'>;
type FixedSpeechPresetUpdate = FixedSpeechPresetDraft & Pick<FixedSpeechPreset, 'id'>;
type StorageLike = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function resolveStorage(storage?: StorageLike): StorageLike | null {
  if (storage) return storage;
  if (typeof window !== 'undefined' && window.localStorage) return window.localStorage;
  return null;
}

function countUnicodeCharacters(value: string): number {
  return Array.from(value).length;
}

function normalizeFields(input: FixedSpeechPresetDraft): FixedSpeechPresetDraft {
  const title = input.title.trim();
  const text = input.text.trim();
  if (!title) throw new Error('标题不能为空');
  if (countUnicodeCharacters(title) > MAX_TITLE_LENGTH) throw new Error('标题最多 80 个字符');
  if (!text) throw new Error('文本不能为空');
  if (countUnicodeCharacters(text) > MAX_TEXT_LENGTH) throw new Error('文本最多 500 个字符');
  return { title, text };
}

function normalizeRecord(value: unknown): FixedSpeechPreset | null {
  if (!value || typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;
  if (
    typeof record.id !== 'string'
    || typeof record.title !== 'string'
    || typeof record.text !== 'string'
    || typeof record.createdAt !== 'string'
    || typeof record.updatedAt !== 'string'
  ) return null;
  try {
    const fields = normalizeFields({ title: record.title, text: record.text });
    const id = record.id.trim();
    return id ? { id, ...fields, createdAt: record.createdAt, updatedAt: record.updatedAt } : null;
  } catch {
    return null;
  }
}

function normalizeList(value: unknown): FixedSpeechPreset[] | null {
  if (!Array.isArray(value) || value.length > MAX_PRESET_COUNT) return null;
  const presets = value.map(normalizeRecord);
  if (presets.some((preset) => preset === null)) return null;
  const normalized = presets as FixedSpeechPreset[];
  return new Set(normalized.map(({ id }) => id)).size === normalized.length ? normalized : null;
}

function parsePresets(raw: string): FixedSpeechPreset[] | null {
  try {
    return normalizeList(JSON.parse(raw));
  } catch {
    return null;
  }
}

function createPresetId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  return `fixed-speech-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

export function loadFixedSpeechPresets(storage?: StorageLike): FixedSpeechPreset[] {
  const target = resolveStorage(storage);
  if (!target) return [];
  const current = target.getItem(FIXED_SPEECH_PRESETS_STORAGE_KEY);
  if (current !== null) return parsePresets(current) ?? [];
  const legacy = target.getItem(LEGACY_PRESETS_STORAGE_KEY);
  if (legacy === null) return [];
  const migrated = parsePresets(legacy);
  if (!migrated) return [];
  target.setItem(FIXED_SPEECH_PRESETS_STORAGE_KEY, JSON.stringify(migrated));
  target.removeItem(LEGACY_PRESETS_STORAGE_KEY);
  return migrated;
}

export function saveFixedSpeechPresets(
  storage: StorageLike | undefined,
  presets: FixedSpeechPreset[],
): FixedSpeechPreset[] {
  const normalized = normalizeList(presets);
  if (!normalized) throw new Error('预制文本格式无效');
  const target = resolveStorage(storage);
  if (!target) return normalized;
  if (normalized.length === 0) target.removeItem(FIXED_SPEECH_PRESETS_STORAGE_KEY);
  else target.setItem(FIXED_SPEECH_PRESETS_STORAGE_KEY, JSON.stringify(normalized));
  return normalized;
}

export function addFixedSpeechPreset(
  storage: StorageLike | undefined,
  presets: FixedSpeechPreset[],
  draft: FixedSpeechPresetDraft,
  now = new Date().toISOString(),
): FixedSpeechPreset[] {
  if (presets.length >= MAX_PRESET_COUNT) throw new Error('最多保存 10 条预制文本');
  const fields = normalizeFields(draft);
  return saveFixedSpeechPresets(storage, [
    ...presets,
    { id: createPresetId(), ...fields, createdAt: now, updatedAt: now },
  ]);
}

export function updateFixedSpeechPreset(
  storage: StorageLike | undefined,
  presets: FixedSpeechPreset[],
  update: FixedSpeechPresetUpdate,
  now = new Date().toISOString(),
): FixedSpeechPreset[] {
  const fields = normalizeFields(update);
  if (!presets.some(({ id }) => id === update.id)) throw new Error('未找到要更新的预制文本');
  return saveFixedSpeechPresets(storage, presets.map((preset) => preset.id === update.id
    ? { ...preset, ...fields, updatedAt: now }
    : preset));
}

export function removeFixedSpeechPreset(
  storage: StorageLike | undefined,
  presets: FixedSpeechPreset[],
  id: string,
): FixedSpeechPreset[] {
  return saveFixedSpeechPresets(storage, presets.filter((preset) => preset.id !== id));
}
