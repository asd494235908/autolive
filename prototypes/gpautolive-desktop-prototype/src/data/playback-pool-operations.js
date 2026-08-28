export const PLAYBACK_POOL_LIMIT = 100;

const SUPPORTED_EXTENSIONS = new Set([
  "mp4", "mov", "mkv", "avi", "webm", "m4v", "ts", "m2ts", "flv", "wmv", "3gp",
  "mp3", "wav", "m4a", "aac", "ogg", "flac",
]);

function fileExtension(name) {
  return String(name).split(".").pop()?.toLowerCase() ?? "";
}

function duplicateKey(item) {
  return String(item.duplicateKey ?? item.name).trim().toLowerCase();
}

function fail(error) {
  return { ok: false, error };
}

export function preparePoolFiles(files) {
  const sourceFiles = [...(files ?? [])];
  if (!sourceFiles.length) return fail("没有选择媒体，播放池保持不变");
  if (sourceFiles.length > PLAYBACK_POOL_LIMIT) return fail(`一次最多选择 ${PLAYBACK_POOL_LIMIT} 个媒体`);

  const seen = new Set();
  const items = [];
  for (const file of sourceFiles) {
    const key = String(file.name ?? "").trim().toLowerCase();
    if (!key || !SUPPORTED_EXTENSIONS.has(fileExtension(key))) return fail(`不支持的媒体格式：${file.name || "未命名文件"}`);
    if (!Number.isFinite(file.size) || file.size <= 0) return fail(`媒体文件为空：${file.name}`);
    const identity = `${key}\u0000${file.size}\u0000${file.lastModified ?? 0}`;
    if (seen.has(identity)) return fail(`本次选择包含重复文件：${file.name}`);
    seen.add(identity);
    items.push({
      id: `local:${encodeURIComponent(key)}:${file.size}:${file.lastModified ?? 0}`,
      duplicateKey: identity,
      name: file.name,
      meta: `待探测 · 本地文件 · ${(file.size / 1024 / 1024).toFixed(1)} MB`,
      status: "ready",
    });
  }
  return { ok: true, items };
}

export function replacePlaybackPool(items) {
  return { ok: true, changed: true, pool: items };
}

export function appendPlaybackPool(pool, items) {
  if (pool.length + items.length > PLAYBACK_POOL_LIMIT) return fail(`播放池最多包含 ${PLAYBACK_POOL_LIMIT} 项`);
  const existing = new Set(pool.map(duplicateKey));
  const duplicate = items.find((item) => existing.has(duplicateKey(item)));
  if (duplicate) return fail(`播放池中已存在：${duplicate.name}`);
  return { ok: true, changed: items.length > 0, pool: [...pool, ...items] };
}

export function replacePlaybackPoolItem(pool, targetId, item) {
  const targetIndex = pool.findIndex((entry) => entry.id === targetId);
  if (targetIndex < 0) return fail("要替换的播放池条目已不存在");
  const duplicate = pool.find((entry, index) => index !== targetIndex && duplicateKey(entry) === duplicateKey(item));
  if (duplicate) return fail(`播放池中已存在：${item.name}`);
  if (duplicateKey(pool[targetIndex]) === duplicateKey(item)) return { ok: true, changed: false, pool };
  const nextPool = [...pool];
  nextPool[targetIndex] = item;
  return { ok: true, changed: true, pool: nextPool };
}

export function movePlaybackPoolItem(pool, itemId, targetIndex) {
  const sourceIndex = pool.findIndex((entry) => entry.id === itemId);
  if (sourceIndex < 0 || targetIndex < 0 || targetIndex >= pool.length) return fail("播放池排序位置无效");
  if (sourceIndex === targetIndex) return { ok: true, changed: false, pool };
  const nextPool = [...pool];
  const [item] = nextPool.splice(sourceIndex, 1);
  nextPool.splice(targetIndex, 0, item);
  return { ok: true, changed: true, pool: nextPool };
}

export function removePlaybackPoolItem(pool, itemId) {
  if (!pool.some((entry) => entry.id === itemId)) return fail("要删除的播放池条目已不存在");
  return { ok: true, changed: true, pool: pool.filter((entry) => entry.id !== itemId) };
}
