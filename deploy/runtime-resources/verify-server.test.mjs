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
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';

import { verifyResourceServer } from './verify-server.mjs';

const deployRoot = dirname(new URL(import.meta.url).pathname);
const release = 'v0.1.0';
const target = 'aarch64-apple-darwin';
const testGroup = spawnSync('id', ['-gn'], { encoding: 'utf8' }).stdout.trim();
const testGroupId = Number(spawnSync('id', ['-g'], { encoding: 'utf8' }).stdout.trim());

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

function stagingRoot(root) {
  return join(root, 'staging');
}

function publishedRoot(root) {
  return join(root, 'published');
}

function copyScriptForTest(root, scriptName, replacements) {
  let source = readFileSync(join(deployRoot, scriptName), 'utf8');
  for (const [from, to] of replacements) source = source.replaceAll(from, to);
  const destination = join(root, 'scripts', scriptName);
  mkdirSync(dirname(destination), { recursive: true });
  writeFileSync(destination, source);
  chmodSync(destination, 0o755);
  return destination;
}

function publisherFixture(root, { deviceMismatch = false } = {}) {
  const flock = writeFile(root, 'bin/flock', '#!/bin/sh\n[ "$1" = -x ] && [ "$2" = 9 ] || exit 97\nexit 0\n');
  const mv = writeFile(root, 'bin/mv', '#!/bin/sh\n[ "$1" = -T ] && [ "$2" = -- ] || exit 98\nshift 2\nexec /bin/mv "$@"\n');
  const stat = writeFile(
    root,
    'bin/stat',
    deviceMismatch
      ? '#!/bin/sh\ncase "$3" in *published*) printf "2\\n" ;; *) printf "1\\n" ;; esac\n'
      : '#!/bin/sh\nprintf "1\\n"\n',
  );
  const python = spawnSync('/bin/sh', ['-c', 'command -v python3'], { encoding: 'utf8' }).stdout.trim();
  assert.ok(python);
  for (const path of [flock, mv, stat]) chmodSync(path, 0o755);
  const publisher = copyScriptForTest(root, 'publish-runtime-resources', [
    ['/fs/autolive-resources-staging', stagingRoot(root)],
    ['/fs/autolive-resources', publishedRoot(root)],
    ['/var/lock/autolive-resources', join(root, 'locks')],
    ['/usr/bin/flock', flock],
    ['/usr/bin/stat', stat],
    ['/usr/bin/mv', mv],
    ['/usr/bin/mkdir', '/bin/mkdir'],
    ['/usr/bin/rm', '/bin/rm'],
    ['/usr/bin/python3', python],
    ['/usr/bin/chmod', '/bin/chmod'],
    ['PUBLISH_GROUP=autolive-resources', `PUBLISH_GROUP=${testGroup}`],
  ]);
  return { publisher, staging: stagingRoot(root), published: publishedRoot(root) };
}

function prepareScope(root, uploadId, scope, files, inventoryOverrides = {}) {
  const scopeRoot = join(stagingRoot(root), uploadId, release, scope);
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

function runPublisher(fixture, scope, uploadId) {
  return runShell(fixture.publisher, [release, scope, uploadId]);
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
  let redirectRequests = 0;
  let followedRedirects = 0;
  const server = createServer((request, response) => {
    if (request.url === '/redirect/common/media.bin') {
      redirectRequests += 1;
      response.writeHead(302, { Location: '/common/media.bin' }).end();
      return;
    }
    if (request.url === '/common/media.bin') followedRedirects += 1;
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
    redirectRequests = 0;
    followedRedirects = 0;
    await assert.rejects(
      verifyResourceServer({
        baseUrl: `http://127.0.0.1:${port}/redirect/`,
        manifest: { ...manifest, files: [{ ...manifest.files[0], sha256: sha256(body) }] },
        sampleRelativePath: 'common/media.bin',
      }),
    );
    assert.equal(redirectRequests, 1);
    assert.equal(followedRedirects, 0);
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
    'LogsDirectory=autolive-resources',
    'RequiresMountsFor=/fs/autolive-resources /fs/autolive-resources-staging',
    'Restart=on-failure',
    'WantedBy=multi-user.target',
  ]) assert.match(unit, new RegExp(line.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
  const unitStart = unit.indexOf('[Unit]');
  const serviceStart = unit.indexOf('[Service]');
  const installStart = unit.indexOf('[Install]');
  assert.ok(unitStart >= 0 && serviceStart > unitStart && installStart > serviceStart);
  const unitSection = unit.slice(unitStart, serviceStart);
  const serviceSection = unit.slice(serviceStart, installStart);
  assert.match(unitSection, /RequiresMountsFor=\/fs\/autolive-resources \/fs\/autolive-resources-staging/);
  assert.doesNotMatch(serviceSection, /RequiresMountsFor=/);
});

test('部署身份使用锁定密码的 /bin/sh，并依赖 forced-command restrict', () => {
  const readme = readFileSync(join(deployRoot, 'README.md'), 'utf8');
  assert.match(readme, /useradd[^\n]*--shell \/bin\/sh[^\n]*autolive-deploy/);
  assert.match(readme, /usermod --shell \/bin\/sh autolive-deploy/);
  assert.match(readme, /passwd -l autolive-deploy/);
  assert.match(readme, /PasswordAuthentication no/);
  assert.match(readme, /KbdInteractiveAuthentication no/);
  assert.match(readme, /PermitTTY no/);
  assert.match(readme, /AllowAgentForwarding no/);
  assert.match(readme, /AllowTcpForwarding no/);
  assert.match(readme, /X11Forwarding no/);
  assert.match(readme, /command="\/usr\/local\/sbin\/autolive-resource-deploy-dispatcher",restrict/);
  assert.match(readme, /restrict[^\n]*(?:PTY|终端)[^\n]*(?:转发|forwarding)/i);
  assert.match(readme, /不要使用[^\n]*\/usr\/sbin\/nologin/);
});

test('dispatcher 只允许受限 rsync 和精确 publisher 命令', () => {
  const root = testRoot();
  const rrsyncMarker = join(root, 'rrsync.called');
  const publisherMarker = join(root, 'publisher.called');
  const rrsync = writeFile(root, 'fake/rrsync', `#!/bin/sh\nprintf "%s\\n%s" "$SSH_ORIGINAL_COMMAND" "$*" > '${rrsyncMarker}'\n`);
  const publisher = writeFile(root, 'fake/publisher', `#!/bin/sh\nprintf "%s\\n" "$*" > '${publisherMarker}'\n`);
  for (const path of [rrsync, publisher]) chmodSync(path, 0o755);
  const dispatcher = copyScriptForTest(root, 'autolive-resource-deploy-dispatcher', [
    ['/usr/local/lib/autolive-resources/rrsync', rrsync],
    ['/usr/local/sbin/publish-runtime-resources', publisher],
    ['/fs/autolive-resources-staging', stagingRoot(root)],
  ]);

  let result = runShell(dispatcher, [], { SSH_ORIGINAL_COMMAND: 'rsync --server -logDtpre.iLsfxC . 12-3/v0.1.0/common/' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(readFileSync(rrsyncMarker, 'utf8'), /-wo -no-overwrite -munge/);

  result = runShell(dispatcher, [], { SSH_ORIGINAL_COMMAND: 'publish-runtime-resources v0.1.0 common 12-3' });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(publisherMarker, 'utf8'), 'v0.1.0 common 12-3\n');

  for (const command of [
    'echo owned',
    ' rsync --server -logDtpre.iLsfxC . 12-3/v0.1.0/common/',
    'rsync  --server -logDtpre.iLsfxC . 12-3/v0.1.0/common/',
    'rsync\t--server -logDtpre.iLsfxC . 12-3/v0.1.0/common/',
    'rsync --server -logDtpre.iLsfxC . 12-3/v0.1.0/common/ ',
    'rsync --server -logDtpre.iLsfxC . 12-3/v0.1.0/common/;touch pwned',
    'rsync --server -logDtpre.iLsfxC . 12-3/v0.2.0/common/',
    'rsync --server -logDtpre.iLsfxC . 12-3/v0.1.0/../../',
    'publish-runtime-resources v0.1.0 common 12-3 extra',
    'publish-runtime-resources v0.1.0 common 0-3',
    'publish-runtime-resources v0.1.0 common 12-0',
    'publish-runtime-resources v0.1.0 bad-scope 12-3',
  ]) {
    result = runShell(dispatcher, [], { SSH_ORIGINAL_COMMAND: command });
    assert.equal(result.status, 64, `${command}: ${result.stderr}`);
  }
  assert.equal(existsSync(join(root, 'pwned')), false);
  const productionDispatcher = readFileSync(join(deployRoot, 'autolive-resource-deploy-dispatcher'), 'utf8');
  assert.doesNotMatch(productionDispatcher, /eval/);
  assert.doesNotMatch(productionDispatcher, /AUTOLIVE_TEST_MODE|TEST_ROOT|FLOCK_BIN/);
  assert.doesNotMatch(productionDispatcher, /\/usr\/bin\/rrsync/);
});

test('publisher 校验 inventory、拒绝额外文件和 symlink，并原子发布', () => {
  const root = testRoot();
  const fixture = publisherFixture(root);
  const { publisher } = fixture;
  const first = prepareScope(root, '12-3', target, { 'binaries/ffmpeg': 'media-binary' });
  let result = runPublisher(fixture, target, '12-3');
  assert.equal(result.status, 0, result.stderr);
  const destination = join(fixture.published, 'autolive-resources', release, target);
  assert.equal(readFileSync(join(destination, 'binaries/ffmpeg'), 'utf8'), 'media-binary');
  assert.equal(existsSync(first), false);
  assert.equal(existsSync(join(destination, 'autolive-deploy-inventory.json')), false);
  const destinationDirMode = statSync(destination).mode;
  const destinationStat = statSync(destination);
  const destinationFileStat = statSync(join(destination, 'binaries/ffmpeg'));
  const destinationFileMode = destinationFileStat.mode;
  assert.equal(destinationStat.gid, testGroupId);
  assert.equal(destinationFileStat.gid, testGroupId);
  assert.equal(destinationDirMode & 0o040, 0o040);
  assert.equal(destinationDirMode & 0o010, 0o010);
  assert.equal(destinationFileMode & 0o040, 0o040);
  assert.equal(destinationFileMode & 0o007, 0);

  prepareScope(root, '12-4', target, { 'binaries/ffmpeg': 'media-binary' });
  result = runPublisher(fixture, target, '12-4');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(existsSync(join(fixture.staging, '.candidates', `12-4-${release}-${target}`)), false);

  prepareScope(root, '12-5', target, { 'binaries/ffmpeg': 'different' });
  result = runPublisher(fixture, target, '12-5');
  assert.equal(result.status, 65);
  assert.equal(existsSync(join(fixture.staging, '.candidates', `12-5-${release}-${target}`)), true);

  const extraRoot = prepareScope(root, '12-6', target, { 'binaries/ffmpeg': 'media-binary' });
  writeFile(extraRoot, 'unexpected.txt', 'unexpected');
  result = runPublisher(fixture, target, '12-6');
  assert.equal(result.status, 65);

  const symlinkRoot = prepareScope(root, '12-7', target, { 'binaries/ffmpeg': 'media-binary' });
  symlinkSync('ffmpeg', join(symlinkRoot, 'binaries', 'alias'));
  result = runPublisher(fixture, target, '12-7');
  assert.equal(result.status, 65);
  assert.equal(lstatSync(join(fixture.staging, '.candidates', `12-7-${release}-${target}`)).isDirectory(), true);

  const missingInventoryRoot = prepareScope(root, '12-8', target, { 'binaries/ffmpeg': 'media-binary' });
  rmSync(join(missingInventoryRoot, 'autolive-deploy-inventory.json'));
  result = runPublisher(fixture, target, '12-8');
  assert.equal(result.status, 65);

  const invalidInventoryRoot = prepareScope(root, '12-9', target, { 'binaries/ffmpeg': 'media-binary' });
  writeFile(invalidInventoryRoot, 'autolive-deploy-inventory.json', '{"schema":2}');
  result = runPublisher(fixture, target, '12-9');
  assert.equal(result.status, 65);

  const oversizedRoot = prepareScope(root, '12-10', target, { 'binaries/ffmpeg': 'media-binary' });
  const oversizedInventory = JSON.parse(readFileSync(join(oversizedRoot, 'autolive-deploy-inventory.json'), 'utf8'));
  oversizedInventory.files[0].size_bytes = 64 * 1024 * 1024 * 1024 + 1;
  writeFile(oversizedRoot, 'autolive-deploy-inventory.json', JSON.stringify(oversizedInventory));
  result = runPublisher(fixture, target, '12-10');
  assert.equal(result.status, 65);

  result = runShell(publisher, [release, target, '0-8']);
  assert.equal(result.status, 64);
});

test('publisher 在固定根设备号不一致时 fail-closed，成功发布使用同设备根', () => {
  const mismatchRoot = testRoot();
  const mismatchFixture = publisherFixture(mismatchRoot, { deviceMismatch: true });
  const source = prepareScope(mismatchRoot, '13-1', target, { 'binaries/ffmpeg': 'media-binary' });
  const result = runPublisher(mismatchFixture, target, '13-1');
  assert.equal(result.status, 65);
  assert.equal(existsSync(source), true);
  assert.equal(existsSync(join(mismatchFixture.published, 'autolive-resources', release, target)), false);
  const productionPublisher = readFileSync(join(deployRoot, 'publish-runtime-resources'), 'utf8');
  assert.match(productionPublisher, /stat -c %d/);
  assert.match(productionPublisher, /mv -T --/);
  assert.match(productionPublisher, /PUBLISH_GROUP=autolive-resources/);
  assert.doesNotMatch(productionPublisher, /AUTOLIVE_TEST_MODE|TEST_ROOT|FLOCK_BIN/);
});

test('publisher 先完成有界 Python 校验，再收敛权限，并固定外部工具路径', () => {
  const publisher = readFileSync(join(deployRoot, 'publish-runtime-resources'), 'utf8');
  assert.doesNotMatch(publisher, /\/usr\/bin\/find/);

  const validation = publisher.indexOf('if ! publish_state=$(');
  const chgrp = publisher.indexOf('/usr/bin/chgrp -R "$PUBLISH_GROUP" "$candidate"');
  const chmod = publisher.indexOf('/usr/bin/chmod -R g+rX,o-rwx "$candidate"');
  const removeInventory = publisher.indexOf('rm "$candidate/$inventory_name"');
  assert.ok(validation >= 0);
  assert.ok(chgrp > validation && chmod > chgrp && removeInventory > chmod);

  for (const tool of ['mkdir', 'rm', 'stat', 'mv', 'chgrp', 'chmod', 'flock', 'python3']) {
    assert.match(publisher, new RegExp(`/usr/bin/${tool}`));
  }
  assert.doesNotMatch(publisher, /(^|\n)[\t ]*(?:mkdir|rm|stat|mv|chgrp|chmod|flock|python3)(?=\s|$)/m);
});
