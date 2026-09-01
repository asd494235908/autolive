import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('AkVirtualCamera CPU test-pattern 工具固定受限协议并输出门禁证据', async () => {
  const source = await readFile(
    new URL('../third_party/akvirtualcamera/run-test-pattern.ps1', import.meta.url),
    'utf8',
  );
  assert.match(source, /NamedPipeClientStream/);
  assert.match(source, /PipeDirection\]::Out/);
  assert.match(source, /GPAKVC01/);
  assert.match(source, /Add-U64 \$header \$Generation/);
  assert.match(source, /Add-U64 \$header \$Sequence/);
  assert.match(source, /Add-U32 \$header 1280/);
  assert.match(source, /Add-U32 \$header 720/);
  assert.match(source, /format = 'YUY2'/);
  assert.match(source, /fps = 30/);
  assert.match(source, /sequenceMonotonic = \$sequenceMonotonic/);
  assert.match(source, /必须是绝对路径/);
  assert.match(source, /exit 2/);
});
