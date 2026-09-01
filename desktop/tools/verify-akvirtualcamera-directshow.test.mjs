import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const build = new URL('../third_party/akvirtualcamera/build-directshow.ps1', import.meta.url);

test('DirectShow 构建脚本覆盖 x86/x64 且不安装系统设备', async () => {
  const text = await readFile(build, 'utf8');
  assert.match(text, /ValidateSet\('x86', 'x64'\)/);
  assert.match(text, /VirtualCamera_dshow/);
  assert.match(text, /network = 'none'/);
  assert.match(text, /release_ready = \$false/);
  assert.match(text, /0002-loopback-service-socket\.patch/);
  assert.match(text, /apply --check/);
  assert.match(text, /apply --reverse --check/);
  assert.doesNotMatch(text, /regsvr32|Start-Service|New-Service|git\s+clone/);
});
