export const AUDIO_CYCLE_SNAPSHOT_STORAGE_KEY = 'autolive.audio-cycle-snapshots.v1';
const MAX_SNAPSHOTS = 40;

export type AudioCycleSnapshotRecord = {
  at: string;
  seed: number;
  presetIds: string[];
  weights: number[];
  values: Record<string, number>;
};

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

function resolveStorage(storage?: StorageLike | null): StorageLike | null {
  if (storage) return storage;
  if (typeof window !== 'undefined' && window.localStorage) return window.localStorage;
  return null;
}

export function loadAudioCycleSnapshots(storage?: StorageLike | null): AudioCycleSnapshotRecord[] {
  const target = resolveStorage(storage);
  if (!target) return [];
  try {
    const raw = target.getItem(AUDIO_CYCLE_SNAPSHOT_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((item) => item && typeof item === 'object')
      .map((item) => item as AudioCycleSnapshotRecord)
      .filter((item) => Array.isArray(item.presetIds) && typeof item.seed === 'number')
      .slice(-MAX_SNAPSHOTS);
  } catch {
    return [];
  }
}

export function appendAudioCycleSnapshot(
  record: Omit<AudioCycleSnapshotRecord, 'at'> & { at?: string },
  storage?: StorageLike | null,
): AudioCycleSnapshotRecord[] {
  const target = resolveStorage(storage);
  const next: AudioCycleSnapshotRecord = {
    at: record.at ?? new Date().toISOString(),
    seed: record.seed,
    presetIds: [...record.presetIds],
    weights: [...record.weights],
    values: { ...record.values },
  };
  const list = [...loadAudioCycleSnapshots(target), next].slice(-MAX_SNAPSHOTS);
  if (target) {
    target.setItem(AUDIO_CYCLE_SNAPSHOT_STORAGE_KEY, JSON.stringify(list));
  }
  return list;
}
