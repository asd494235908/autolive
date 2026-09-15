import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('YUY2 DirectShow 声明、分配和实际sample均使用去padding后的大小', async () => {
  const patch = await readFile(new URL('../third_party/akvirtualcamera/patches/0008-directshow-yuy2-packed-stride.patch', import.meta.url), 'utf8');
  assert.match(patch, /biSizeImage = DWORD\(frameSize\)/);
  assert.match(patch, /allocatorRequirements\.cbBuffer = LONG\(videoFormat\.format\(\) == PixelFormat_yuyv422/);
  assert.match(patch, /copyPackedYuy2Sample\(pData, size, frame\.constData\(\), frame\.lineSize\(0\)/);
  assert.match(patch, /SetActualDataLength\(dataLength\)/);
  assert.match(patch, /destination \+ row \* rowBytes, source \+ row \* sourceStride, rowBytes/);
  assert.doesNotMatch(patch, /height % 2/);
  for (const name of ['build-sidecar.ps1', 'build-directshow.ps1']) {
    const build = await readFile(new URL(`../third_party/akvirtualcamera/${name}`, import.meta.url), 'utf8');
    assert.match(build, /0008-directshow-yuy2-packed-stride\.patch/);
    assert.match(build, /windows\/PlatformUtils\/src\/yuy2_sample\.h/);
  }
});
