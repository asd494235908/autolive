export const VOICE_CLONE_PRESETS_STORAGE_KEY = 'autolive.voice-clone-presets.v1';
const MAX_PRESET_COUNT = 10;
const MAX_TEXT_LENGTH = 500;

export type VoiceClonePreset = {
  id: string;
  title: string;
  text: string;
  createdAt: string;
  updatedAt: string;
};

type VoiceClonePresetDraft = {
  title: string;
  text: string;
};

type VoiceClonePresetUpdate = VoiceClonePresetDraft & {
  id: string;
};

type StorageLike = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function resolveStorage(storage?: StorageLike) {
  if (storage) return storage;
  if (typeof window !== 'undefined' && window.localStorage) return window.localStorage;
  return null;
}

function getNow(now?: string) {
  return now ?? new Date().toISOString();
}

function countUnicodeCharacters(value: string) {
  return Array.from(value).length;
}

function normalizePresetFields(input: VoiceClonePresetDraft) {
  const title = input.title.trim();
  const text = input.text.trim();
  if (!title) throw new Error('标题不能为空');
  if (!text) throw new Error('文本不能为空');
  if (countUnicodeCharacters(text) > MAX_TEXT_LENGTH) {
    throw new Error('文本最多 500 个字符');
  }
  return { title, text };
}

function normalizePresetRecord(value: unknown): VoiceClonePreset | null {
  if (!value || typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;
  if (
    typeof record.id !== 'string' ||
    typeof record.title !== 'string' ||
    typeof record.text !== 'string' ||
    typeof record.createdAt !== 'string' ||
    typeof record.updatedAt !== 'string'
  ) {
    return null;
  }
  try {
    const normalized = normalizePresetFields({ title: record.title, text: record.text });
    return {
      id: record.id.trim(),
      title: normalized.title,
      text: normalized.text,
      createdAt: record.createdAt,
      updatedAt: record.updatedAt,
    };
  } catch {
    return null;
  }
}

function normalizePresetList(presets: VoiceClonePreset[]) {
  if (!Array.isArray(presets)) throw new Error('预制文本格式无效');
  if (presets.length > MAX_PRESET_COUNT) throw new Error('最多保存 10 条预制文本');
  const seenIds = new Set<string>();
  return presets.map((preset) => {
    const normalized = normalizePresetRecord(preset);
    if (!normalized || !normalized.id) throw new Error('预制文本格式无效');
    if (seenIds.has(normalized.id)) throw new Error('预制文本 ID 重复');
    seenIds.add(normalized.id);
    return normalized;
  });
}

function createPresetId() {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `voice-clone-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

export function loadVoiceClonePresets(storage?: StorageLike) {
  const target = resolveStorage(storage);
  if (!target) return [] as VoiceClonePreset[];
  const raw = target.getItem(VOICE_CLONE_PRESETS_STORAGE_KEY);
  if (!raw) return [] as VoiceClonePreset[];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [] as VoiceClonePreset[];
    const normalized = parsed.map(normalizePresetRecord).filter((item): item is VoiceClonePreset => item !== null);
    if (normalized.length !== parsed.length || normalized.length > MAX_PRESET_COUNT) return [] as VoiceClonePreset[];
    return normalized;
  } catch {
    return [] as VoiceClonePreset[];
  }
}

export function saveVoiceClonePresets(storage: StorageLike | undefined, presets: VoiceClonePreset[]) {
  const target = resolveStorage(storage);
  const normalized = normalizePresetList(presets);
  if (!target) return normalized;
  if (normalized.length === 0) {
    target.removeItem(VOICE_CLONE_PRESETS_STORAGE_KEY);
    return normalized;
  }
  target.setItem(VOICE_CLONE_PRESETS_STORAGE_KEY, JSON.stringify(normalized));
  return normalized;
}

export function addVoiceClonePreset(
  storage: StorageLike | undefined,
  presets: VoiceClonePreset[],
  draft: VoiceClonePresetDraft,
  now?: string,
) {
  if (presets.length >= MAX_PRESET_COUNT) {
    throw new Error('最多保存 10 条预制文本');
  }
  const normalized = normalizePresetFields(draft);
  return saveVoiceClonePresets(storage, [
    ...normalizePresetList(presets),
    {
      id: createPresetId(),
      title: normalized.title,
      text: normalized.text,
      createdAt: getNow(now),
      updatedAt: getNow(now),
    },
  ]);
}

export function updateVoiceClonePreset(
  storage: StorageLike | undefined,
  presets: VoiceClonePreset[],
  update: VoiceClonePresetUpdate,
  now?: string,
) {
  const normalized = normalizePresetFields(update);
  const current = normalizePresetList(presets);
  const currentPreset = current.find((preset) => preset.id === update.id);
  if (!currentPreset) throw new Error('未找到要更新的预制文本');
  const nextUpdatedAt = getNow(now);
  return saveVoiceClonePresets(
    storage,
    current.map((preset) =>
      preset.id === update.id
        ? {
            ...preset,
            title: normalized.title,
            text: normalized.text,
            updatedAt: nextUpdatedAt,
          }
        : preset,
    ),
  );
}

export function removeVoiceClonePreset(
  storage: StorageLike | undefined,
  presets: VoiceClonePreset[],
  id: string,
) {
  return saveVoiceClonePresets(
    storage,
    normalizePresetList(presets).filter((preset) => preset.id !== id),
  );
}
