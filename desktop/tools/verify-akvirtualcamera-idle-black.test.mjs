import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('DirectShow 无生产者黑帧仅限 direct-mode YUY2，保留非 direct 行为', async () => {
  const patch = await readFile(new URL('../third_party/akvirtualcamera/patches/0006-directshow-yuy2-idle-black.patch', import.meta.url), 'utf8');
  assert.match(patch, /!isActive && format\.format\(\) == PixelFormat_yuyv422/);
  assert.match(patch, /m_directMode && format\.format\(\) == PixelFormat_yuyv422/);
  assert.match(patch, /fillYuy2BlackFrame\(frame\.data\(\), frame\.size\(\)\);\n\+        return frame;/);
  assert.match(patch, /index % 2 == 0 \? 16 : 128/);
  assert.doesNotMatch(patch, /^-.*(?:std::generate|m_videoAdjusts\.adjust)/m);
});
