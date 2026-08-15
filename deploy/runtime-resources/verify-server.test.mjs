import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { createServer } from 'node:http';
import { once } from 'node:events';
import {
  chmodSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';

import { verifyResourceServer } from './verify-server.mjs';

const deployRoot = dirname(new URL(import.meta.url).pathname);
const release = 'v0.1.0';
const target = 'aarch64-apple-darwin';

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

function writeFile(root, relativePath, content) {
  const path = join(root, ...relativePath.split('/'));
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  return path;
}

function runShell(script, args, env = {}) {
  return spawnSync('/bin/sh', [script, ...args], {
    encoding: 'utf8',
    env: { ...process.env, ...env },
  });
}

function testRoot() {
  return mkdtempSync(join(tmpdir(), 'autolive-runtime-deploy-'));
}

function prepareScope(root, uploadId, scope, files, inventoryOverrides = {}) {
  const scopeRoot = join(root, 'autolive-resources-staging', uploadId, release, scope);
  for (const [relativePath, content] of Object.entries(files)) writeFile(scopeRoot, relativePath, content);
  const inventory = {
    schema: 1,
    release,
    scope,
    files: Object.entries(files)
      .map(([relativePath, content]) => ({
        relative_path: relativePath,
        type: 'regular-file',
        size_bytes: Buffer.byteLength(content),
        sha256: sha256(content),
      }))
      .sort((left, right) => left.relative_path.localeCompare(right.relative_path)),
    ...inventoryOverrides,
  };
  writeFile(scopeRoot, 'autolive-deploy-inventory.json', `${JSON.stringify(inventory)}\n`);
  return scopeRoot;
}

function runPublisher(root, scope, uploadId) {
  const flock = writeFile(root, 'bin/flock', '#!/bin/sh\nexit 0\n');
  chmodSync(flock, 0o755);
  return runShell(join(deployRoot, 'publish-runtime-resources'), [release, scope, uploadId], {
    AUTOLIVE_TEST_MODE: '1',
    TEST_ROOT: root,
    FLOCK_BIN: flock,
  });
}

test('验证器要求 HEAD、完整 GET、206 Range、416 越界 Range 和整文件哈希全部成功', async () => {
  const body = Buffer.from('0123456789abcdef-resource-payload');
  const server = createServer((request, response) => {
    if (request.url !== '/autolive-resources/v0.1.0/common/media.bin') {
      response.writeHead(404).end();
      return;
    }
    if (!['HEAD', 'GET'].includes(request.method)) {
      response.writeHead(405).end();
      return;
    }
    if (request.headers.range === 'bytes=0-15') {
      const chunk = body.subarray(0, 16);
      response.writeHead(206, {
        'Content-Length': chunk.length,
        'Content-Range': `bytes 0-15/${body.length}`,
      }).end(request.method === 'HEAD' ? undefined : chunk);
      return;
    }
    if (request.headers.range === `bytes=${body.length}-`) {
      response.writeHead(416, { 'Content-Range': `bytes */${body.length}` }).end();
      return;
    }
    response.writeHead(200, { 'Content-Length': body.length }).end(request.method === 'HEAD' ? undefined : body);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address();
  const manifest = {
    schema_version: 1,
    release,
    files: [{ relative_path: 'common/media.bin', size_bytes: body.length, sha256: sha256(body) }],
  };

  try {
    const result = await verifyResourceServer({
      baseUrl: `http://127.0.0.1:${port}/autolive-resources/v0.1.0/`,
      manifest,
      sampleRelativePath: 'common/media.bin',
    });
    assert.deepEqual(result, { head: true, get: true, range: true, hash: true });
  } finally {
    server.close();
    await once(server, 'close');
  }
});

test('验证器拒绝错误整文件哈希和重定向', async () => {
  const body = Buffer.alloc(32, 0x41);
  const server = createServer((request, response) => {
    if (request.url === '/redirect/') {
      response.writeHead(302, { Location: '/autolive-resources/v0.1.0/common/media.bin' }).end();
      return;
    }
    response.writeHead(200, { 'Content-Length': body.length }).end(body);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address();
  const manifest = {
    schema_version: 1,
    release,
    files: [{ relative_path: 'common/media.bin', size_bytes: body.length, sha256: sha256('wrong') }],
  };

  try {
    await assert.rejects(
      verifyResourceServer({
        baseUrl: `http://127.0.0.1:${port}/`,
        manifest,
        sampleRelativePath: 'common/media.bin',
      }),
      /sha256|哈希/i,
    );
    await assert.rejects(
      verifyResourceServer({
        baseUrl: `http://127.0.0.1:${port}/redirect/`,
        manifest: { ...manifest, files: [{ ...manifest.files[0], sha256: sha256(body) }] },
        sampleRelativePath: 'common/media.bin',
      }),
    );
  } finally {
    server.close();
    await once(server, 'close');
  }
});

test('Caddy 和 systemd 契约固定只读静态服务与收敛权限', () => {
  const caddy = readFileSync(join(deployRoot, 'Caddyfile'), 'utf8');
  assert.match(caddy, /:7088/);
  assert.match(caddy, /method GET HEAD/);
  assert.match(caddy, /root \* \/fs\/autolive-resources/);
  assert.match(caddy, /Cache-Control "public, max-age=31536000, immutable"/);
  assert.match(caddy, /respond 405/);
  assert.match(caddy, /roll_size 50MiB/);
  assert.doesNotMatch(caddy, /browse/);

  const unit = readFileSync(join(deployRoot, 'autolive-resources.service'), 'utf8');
  for (const line of [
    'After=network-online.target',
    'Wants=network-online.target',
    'User=autolive-resources',
    'Group=autolive-resources',
    'NoNewPrivileges=true',
    'PrivateTmp=true',
    'ProtectSystem=strict',
    'ProtectHome=true',
    'ReadOnlyPaths=/fs/autolive-resources',
    'ReadWritePaths=/var/log/autolive-resources',
    'Restart=on-failure',
    'WantedBy=multi-user.target',
  ]) assert.match(unit, new RegExp(line.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
});

test('dispatcher 只允许受限 rsync 和精确 publisher 命令', () => {
  const root = testRoot();
  const rrsync = writeFile(root, 'usr/local/lib/autolive-resources/rrsync', '#!/bin/sh\nprintf "%s\\n%s" "$SSH_ORIGINAL_COMMAND" "$*" > "$TEST_ROOT/rrsync.called"\n');
  const publisher = writeFile(root, 'usr/local/sbin/publish-runtime-resources', '#!/bin/sh\nprintf "%s\\n" "$*" > "$TEST_ROOT/publisher.called"\n');
  for (const path of [rrsync, publisher]) chmodSync(path, 0o755);
  const flock = writeFile(root, 'bin/flock', '#!/bin/sh\nexit 0\n');
  chmodSync(flock, 0o755);
  const dispatcher = join(deployRoot, 'autolive-resource-deploy-dispatcher');
  const env = { AUTOLIVE_TEST_MODE: '1', TEST_ROOT: root, FLOCK_BIN: flock };

  let result = runShell(dispatcher, [], { ...env, SSH_ORIGINAL_COMMAND: 'rsync --server -logDtpre.iLsfxC . 12-3/v0.1.0/common/' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(readFileSync(join(root, 'rrsync.called'), 'utf8'), /-wo -no-overwrite -munge/);

  result = runShell(dispatcher, [], { ...env, SSH_ORIGINAL_COMMAND: 'publish-runtime-resources v0.1.0 common 12-3' });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(join(root, 'publisher.called'), 'utf8'), 'v0.1.0 common 12-3\n');

  for (const command of [
    'echo owned',
    'rsync --server -logDtpre.iLsfxC . 12-3/v0.1.0/common/;touch pwned',
    'rsync --server -logDtpre.iLsfxC . 12-3/v0.2.0/common/',
    'rsync --server -logDtpre.iLsfxC . 12-3/v0.1.0/../../',
    'publish-runtime-resources v0.1.0 common 12-3 extra',
    'publish-runtime-resources v0.1.0 common 0-3',
    'publish-runtime-resources v0.1.0 common 12-0',
    'publish-runtime-resources v0.1.0 bad-scope 12-3',
  ]) {
    result = runShell(dispatcher, [], { ...env, SSH_ORIGINAL_COMMAND: command });
    assert.equal(result.status, 64, `${command}: ${result.stderr}`);
  }
  assert.equal(existsSync(join(root, 'pwned')), false);
  assert.doesNotMatch(readFileSync(join(deployRoot, 'autolive-resource-deploy-dispatcher'), 'utf8'), /eval/);
  assert.doesNotMatch(readFileSync(join(deployRoot, 'autolive-resource-deploy-dispatcher'), 'utf8'), /\/usr\/bin\/rrsync/);
});

test('publisher 校验 inventory、拒绝额外文件和 symlink，并原子发布', () => {
  const root = testRoot();
  const publisher = join(deployRoot, 'publish-runtime-resources');
  const first = prepareScope(root, '12-3', target, { 'binaries/ffmpeg': 'media-binary' });
  let result = runPublisher(root, target, '12-3');
  assert.equal(result.status, 0, result.stderr);
  const destination = join(root, 'autolive-resources', 'autolive-resources', release, target);
  assert.equal(readFileSync(join(destination, 'binaries/ffmpeg'), 'utf8'), 'media-binary');
  assert.equal(existsSync(first), false);
  assert.equal(existsSync(join(destination, 'autolive-deploy-inventory.json')), false);

  prepareScope(root, '12-4', target, { 'binaries/ffmpeg': 'media-binary' });
  result = runPublisher(root, target, '12-4');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(existsSync(join(root, 'autolive-resources-staging', '.candidates', `12-4-${release}-${target}`)), false);

  prepareScope(root, '12-5', target, { 'binaries/ffmpeg': 'different' });
  result = runPublisher(root, target, '12-5');
  assert.equal(result.status, 65);
  assert.equal(existsSync(join(root, 'autolive-resources-staging', '.candidates', `12-5-${release}-${target}`)), true);

  const extraRoot = prepareScope(root, '12-6', target, { 'binaries/ffmpeg': 'media-binary' });
  writeFile(extraRoot, 'unexpected.txt', 'unexpected');
  result = runPublisher(root, target, '12-6');
  assert.equal(result.status, 65);

  const symlinkRoot = prepareScope(root, '12-7', target, { 'binaries/ffmpeg': 'media-binary' });
  symlinkSync('ffmpeg', join(symlinkRoot, 'binaries', 'alias'));
  result = runPublisher(root, target, '12-7');
  assert.equal(result.status, 65);
  assert.equal(lstatSync(join(root, 'autolive-resources-staging', '.candidates', `12-7-${release}-${target}`)).isDirectory(), true);

  const missingInventoryRoot = prepareScope(root, '12-8', target, { 'binaries/ffmpeg': 'media-binary' });
  rmSync(join(missingInventoryRoot, 'autolive-deploy-inventory.json'));
  result = runPublisher(root, target, '12-8');
  assert.equal(result.status, 65);

  const invalidInventoryRoot = prepareScope(root, '12-9', target, { 'binaries/ffmpeg': 'media-binary' });
  writeFile(invalidInventoryRoot, 'autolive-deploy-inventory.json', '{"schema":2}');
  result = runPublisher(root, target, '12-9');
  assert.equal(result.status, 65);

  result = runShell(publisher, [release, target, '0-8'], { AUTOLIVE_TEST_MODE: '1', TEST_ROOT: root });
  assert.equal(result.status, 64);
});
