import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('AkVirtualCamera 签名脚本固定 7 个产物并强制代码签名门禁', async () => {
  const source = await readFile(new URL('../third_party/akvirtualcamera/sign-akvirtualcamera-artifacts.ps1', import.meta.url), 'utf8');
  assert.match(source, /\$lockPath = Join-Path \$RepositoryRoot 'desktop\\third_party\\akvirtualcamera\\upstream\.lock\.json'/);
  assert.match(source, /Get-Content -Raw -LiteralPath \$lockPath/);
  assert.match(source, /\$artifacts\.Count -ne 7/);
  assert.match(source, /1\.3\.6\.1\.5\.5\.7\.3\.3/);
  assert.match(source, /'\/fd', 'SHA256'/);
  assert.match(source, /verify '\/pa' '\/all'/);
  assert.match(source, /Get-AuthenticodeSignature/);
  assert.match(source, /status = 'valid'/);
  assert.match(source, /Move-Item .*signaturePath/);
});
