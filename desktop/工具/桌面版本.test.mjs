import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { readDesktopVersion } from './桌面版本.mjs';

const scriptPath = fileURLToPath(new URL('./桌面版本.mjs', import.meta.url));

function writeConfig(version) {
  const root = mkdtempSync(join(tmpdir(), 'autolive-desktop-version-'));
  const configPath = join(root, 'tauri.conf.json');
  writeFileSync(configPath, `${JSON.stringify({ version })}\n`);
  return configPath;
}

test('桌面版本模块读取当前 Tauri 配置并输出发布版本', () => {
  assert.deepEqual(readDesktopVersion(), { version: '0.1.0', release: 'v0.1.0' });
  assert.deepEqual(readDesktopVersion(writeConfig('1.2.3-alpha.1+build.5')), {
    version: '1.2.3-alpha.1+build.5',
    release: 'v1.2.3-alpha.1+build.5',
  });
});

test('桌面版本模块只接受严格 SemVer', () => {
  for (const invalid of ['01.2.3', '1.02.3', '1.2.03', '1.2.3-01', '1.2.3-', 'v1.2.3']) {
    assert.throws(() => readDesktopVersion(writeConfig(invalid)), /Tauri 版本号无效/);
  }
});

test('桌面版本 CLI 提供 JSON、单字段输出并拒绝未知参数', () => {
  const execute = (args) => spawnSync(process.execPath, [scriptPath, ...args], { encoding: 'utf8' });

  const defaultResult = execute([]);
  assert.equal(defaultResult.status, 0);
  assert.deepEqual(JSON.parse(defaultResult.stdout), { version: '0.1.0', release: 'v0.1.0' });

  const versionResult = execute(['--version']);
  assert.equal(versionResult.status, 0);
  assert.equal(versionResult.stdout, '0.1.0\n');

  const releaseResult = execute(['--release']);
  assert.equal(releaseResult.status, 0);
  assert.equal(releaseResult.stdout, 'v0.1.0\n');

  const invalidResult = execute(['--unknown']);
  assert.notEqual(invalidResult.status, 0);
});
