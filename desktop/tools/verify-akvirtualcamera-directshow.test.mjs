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

test('两个构建入口重提取前只刷新补丁生成的已知 header', async () => {
  for (const name of ['build-sidecar.ps1', 'build-directshow.ps1']) {
    const text = await readFile(new URL(`../third_party/akvirtualcamera/${name}`, import.meta.url), 'utf8');
    const cleanup = text.indexOf('Remove-Item -LiteralPath $generatedHeader');
    const extract = text.indexOf('& $tar.Source -xf');
    assert.match(text, /foreach \(\$headerPath in @\('windows\/dshow\/BaseFilter\/src\/yuy2_black_frame\.h', 'windows\/PlatformUtils\/src\/yuy2_sample\.h'\)\)/);
    assert.match(text, /\$generatedHeader = Join-Path \$sourceStage \('akvirtualcamera-9cf77ae6379e5f635255f4b377478d388a46a3b2\/' \+ \$headerPath\)/);
    assert.match(text, /Test-Path -LiteralPath \$generatedHeader -PathType Leaf/);
    assert.ok(cleanup >= 0 && cleanup < extract, `${name} must refresh before extraction`);
    assert.doesNotMatch(text, /Remove-Item[^\r\n]*-Recurse/);
  }
});
