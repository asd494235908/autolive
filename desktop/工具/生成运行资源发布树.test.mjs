import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { buildRuntimeResourceRelease } from './生成运行资源发布树.mjs';

function writeFixture(root, relativePath, content) {
  const path = join(root, ...relativePath.split('/'));
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, content);
}

test('生成发布副本、裁剪缓存目录并写入精确清单', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-runtime-resources-'));
  const sourceRoot = join(root, 'src-tauri');
  const outputRoot = join(root, 'output');
  const manifestPath = join(root, 'runtime-resources.json');

  writeFixture(sourceRoot, 'binaries/ffmpeg', 'ffmpeg');
  writeFixture(sourceRoot, 'binaries/ffprobe', 'ffprobe');
  writeFixture(sourceRoot, 'voice-worker/autolive-voice-clone-worker', 'worker');
  writeFixture(sourceRoot, 'voice-models/huggingface/hub/model.bin', 'model');
  const chunkedModel = Buffer.alloc(64 * 1024 + 1, 0x5a);
  writeFixture(sourceRoot, 'voice-models/huggingface/hub/large-model.bin', chunkedModel);
  writeFixture(sourceRoot, 'voice-models/huggingface/hub/.locks/active.lock', 'lock');
  writeFixture(sourceRoot, 'voice-models/huggingface/hub/trees/tree.json', 'tree');
  writeFixture(sourceRoot, 'voice-models/huggingface/hub/blobs/blob.bin', 'blob');
  writeFixture(sourceRoot, 'voice-models/huggingface/hub/download.log', 'log');

  const result = buildRuntimeResourceRelease({
    target: 'aarch64-apple-darwin',
    sourceRoot,
    outputRoot,
    manifestPath,
  });

  assert.equal(result.manifest.schema_version, 1);
  assert.equal(result.manifest.release, 'v0.1.0');
  assert.equal(result.manifest.base_url, 'http://101.96.208.132:7088/autolive-resources/v0.1.0/');
  assert.ok(result.manifest.files.every((file) => /^[a-f0-9]{64}$/.test(file.sha256)));
  assert.ok(result.manifest.files.every((file) => !('size' in file)));
  assert.ok(
    result.manifest.files.every((file) =>
      Object.keys(file).sort().join(',') === 'executable,relative_path,sha256,size_bytes',
    ),
  );
  assert.equal(
    existsSync(join(outputRoot, 'autolive-resources/v0.1.0/common/voice-models/huggingface/hub/.locks')),
    false,
  );
  assert.equal(existsSync(join(sourceRoot, 'voice-models/huggingface/hub/.locks')), true);
  assert.deepEqual(
    result.manifest.files.map((file) => file.relative_path),
    [
      'aarch64-apple-darwin/binaries/ffmpeg',
      'aarch64-apple-darwin/binaries/ffprobe',
      'aarch64-apple-darwin/voice-worker/autolive-voice-clone-worker',
      'common/voice-models/huggingface/hub/large-model.bin',
      'common/voice-models/huggingface/hub/model.bin',
    ],
  );
  assert.deepEqual(
    result.manifest.files.map((file) => file.executable),
    [true, true, true, false, false],
  );
  assert.deepEqual(
    result.manifest.files.map((file) => file.size_bytes),
    [6, 7, 6, 64 * 1024 + 1, 5],
  );
  assert.equal(
    result.manifest.files.find((file) => file.relative_path.endsWith('/large-model.bin')).sha256,
    createHash('sha256').update(chunkedModel).digest('hex'),
  );
  assert.deepEqual(JSON.parse(readFileSync(manifestPath, 'utf8')), result.manifest);
});
