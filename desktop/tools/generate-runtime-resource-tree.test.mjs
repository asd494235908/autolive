import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  buildRuntimeResourceRelease,
  RESOURCE_RELEASE,
} from './generate-runtime-resource-tree.mjs';
import { readDesktopVersion } from './desktop-version.mjs';

const DEPLOY_INVENTORY = 'autolive-deploy-inventory.json';
const expectedRelease = readDesktopVersion().release;

function writeFixture(root, relativePath, content) {
  const path = join(root, ...relativePath.split('/'));
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, content);
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

test('只发布当前目标的 FFmpeg 媒体资源', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-runtime-resources-'));
  const sourceRoot = join(root, 'src-tauri');
  const outputRoot = join(root, 'output');
  const manifestPath = join(root, 'runtime-resources.json');
  const largeBinary = Buffer.alloc(64 * 1024 + 1, 0x5a);

  writeFixture(sourceRoot, 'binaries/ffmpeg', largeBinary);
  writeFixture(sourceRoot, 'binaries/ffprobe', 'ffprobe');
  writeFixture(sourceRoot, 'binaries/.gitignore', 'ignored');
  writeFixture(sourceRoot, 'binaries/.hidden/tool', 'ignored');
  writeFixture(sourceRoot, 'binaries/download.log', 'ignored');
  writeFixture(sourceRoot, 'binaries/trees/tree.json', 'ignored');
  mkdirSync(join(sourceRoot, 'binaries/empty'), { recursive: true });

  const result = buildRuntimeResourceRelease({
    target: 'aarch64-apple-darwin',
    sourceRoot,
    outputRoot,
    manifestPath,
  });

  assert.deepEqual(result.manifest.files, [
    {
      relative_path: 'aarch64-apple-darwin/binaries/ffmpeg',
      sha256: sha256(largeBinary),
      size_bytes: largeBinary.length,
      executable: true,
      component: 'media',
    },
    {
      relative_path: 'aarch64-apple-darwin/binaries/ffprobe',
      sha256: sha256('ffprobe'),
      size_bytes: 7,
      executable: true,
      component: 'media',
    },
  ]);
  assert.equal(result.manifest.schema_version, 1);
  assert.equal(RESOURCE_RELEASE, expectedRelease);
  assert.equal(result.manifest.release, expectedRelease);
  assert.equal(result.manifest.target, 'aarch64-apple-darwin');
  assert.equal(
    result.manifest.base_url,
    `http://101.96.208.132:7088/autolive-resources/${expectedRelease}/`,
  );
  assert.deepEqual(JSON.parse(readFileSync(manifestPath, 'utf8')), result.manifest);
  assert.equal(
    readFileSync(join(result.embeddedRoot, 'aarch64-apple-darwin/binaries/ffmpeg')).length,
    largeBinary.length,
  );
  assert.equal(existsSync(join(result.embeddedRoot, 'common')), false);
  assert.equal(existsSync(join(result.releaseRoot, 'common')), false);
  assert.equal(existsSync(join(result.releaseRoot, 'aarch64-apple-darwin/binaries/.hidden')), false);
  assert.equal(existsSync(join(result.releaseRoot, 'aarch64-apple-darwin/binaries/download.log')), false);
  assert.equal(existsSync(join(sourceRoot, 'binaries/.hidden/tool')), true);
  assert.equal(existsSync(join(sourceRoot, 'binaries/download.log')), true);

  const inventory = JSON.parse(
    readFileSync(join(result.releaseRoot, 'aarch64-apple-darwin', DEPLOY_INVENTORY), 'utf8'),
  );
  assert.deepEqual(inventory, {
    schema: 1,
    release: expectedRelease,
    scope: 'aarch64-apple-darwin',
    files: [
      {
        relative_path: 'binaries/ffmpeg',
        type: 'regular-file',
        size_bytes: largeBinary.length,
        sha256: sha256(largeBinary),
      },
      {
        relative_path: 'binaries/ffprobe',
        type: 'regular-file',
        size_bytes: 7,
        sha256: sha256('ffprobe'),
      },
    ],
  });
});

test('发布清单与内嵌资源保留合法空文件', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-runtime-empty-file-'));
  const sourceRoot = join(root, 'src-tauri');
  const outputRoot = join(root, 'output');
  const manifestPath = join(root, 'runtime-resources.json');

  writeFixture(sourceRoot, 'binaries/ffmpeg.exe', 'ffmpeg');
  writeFixture(sourceRoot, 'binaries/ffprobe.exe', '');

  const result = buildRuntimeResourceRelease({
    target: 'x86_64-pc-windows-msvc',
    sourceRoot,
    outputRoot,
    manifestPath,
  });
  const relativePath = 'x86_64-pc-windows-msvc/binaries/ffprobe.exe';

  assert.deepEqual(
    result.manifest.files.find((file) => file.relative_path === relativePath),
    {
      relative_path: relativePath,
      sha256: sha256(''),
      size_bytes: 0,
      executable: true,
      component: 'media',
    },
  );
  assert.equal(readFileSync(join(result.embeddedRoot, ...relativePath.split('/'))).length, 0);
});

test('拒绝未支持的目标三元组', () => {
  assert.throws(
    () => buildRuntimeResourceRelease({ target: 'x86_64-unknown-linux-gnu' }),
    /当前构建平台不受支持/,
  );
});
