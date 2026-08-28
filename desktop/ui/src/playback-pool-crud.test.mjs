import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

async function readSource(path) {
  return readFile(new URL(path, import.meta.url), 'utf8');
}

test('播放池支持系统文件拖入并对监听器做对称清理', async () => {
  const panel = await readSource('./desktop/playback-pool-panel.tsx');

  assert.match(panel, /getCurrentWebview\(\)\.onDragDropEvent/);
  assert.match(panel, /payload\.type === 'drop'/);
  assert.match(panel, /payload\.paths/);
  assert.match(panel, /return \(\) => \{[\s\S]*unlisten\?\.\(\)/);
  assert.doesNotMatch(panel, /\bdraggable=/);
  assert.doesNotMatch(panel, /onDrag(?:Start|Over|End|Enter|Leave|Drop)=/);
});

test('播放池提供追加、替换、排序、删除和清空操作', async () => {
  const [app, panel] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop/playback-pool-panel.tsx'),
  ]);

  for (const command of [
    'append_local_videos',
    'replace_playback_pool_item',
    'reorder_playback_pool_items',
    'remove_playback_pool_item',
    'clear_playback_pool',
  ]) {
    assert.match(app, new RegExp(`'${command}'`));
  }
  assert.match(app, /applyPlaybackPoolSnapshot\(await invokePlaybackSnapshot\(command, args\)\)/);
  assert.match(app, /'replace_playback_pool_item',[\s\S]*source_path: sourcePath, path: selected/);
  assert.match(app, /'reorder_playback_pool_items',[\s\S]*source_paths: sourcePaths/);
  assert.match(app, /'remove_playback_pool_item',[\s\S]*source_path: sourcePath/);
  assert.match(panel, /追加媒体/);
  assert.match(panel, /替换媒体/);
  assert.match(panel, /删除媒体/);
  assert.match(panel, /清空播放池/);
  assert.match(panel, /上移/);
  assert.match(panel, /下移/);
  assert.match(panel, /aria-label="追加媒体"[\s\S]*onClick=\{onAppend\}/);
  assert.equal(panel.match(/error \? <Alert/g)?.length, 1);
  assert.match(app, /snapshotRequestRef\.current \+= 1;[\s\S]*snapshotRefHome\.current = nextSnapshot;[\s\S]*setSnapshot\(nextSnapshot\)/);
  assert.match(app, /currentSource \? `第 \$\{\(snapshot\?\.source_media_index \?\? 0\) \+ 1\}\/\$\{sourceMediaPool\.length\} 项` : '等待导入'/);
});

test('列表排序使用 Pointer Events 并保留键盘操作', async () => {
  const panel = await readSource('./desktop/playback-pool-panel.tsx');

  assert.match(panel, /onPointerDown=/);
  assert.match(panel, /onPointerMove=/);
  assert.match(panel, /onPointerUp=/);
  assert.match(panel, /setPointerCapture/);
  assert.match(panel, /releasePointerCapture/);
  assert.match(panel, /onLostPointerCapture=\{cancelPointerSort\}/);
  assert.match(panel, /aria-label=\{`拖动第 \$\{index \+ 1\} 项排序`\}/);
  assert.match(panel, /aria-label=\{`上移第 \$\{index \+ 1\} 项`\}/);
  assert.match(panel, /aria-label=\{`下移第 \$\{index \+ 1\} 项`\}/);
});

test('所有播放池写操作复用忙碌状态，拖入默认追加且总量仍受 100 项上限保护', async () => {
  const [app, panel] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop/playback-pool-panel.tsx'),
  ]);

  assert.match(app, /const PLAYBACK_POOL_LIMIT = 100/);
  assert.match(app, /if \(sourceMediaPool\.length \+ paths\.length > PLAYBACK_POOL_LIMIT\)/);
  assert.match(app, /onDropFiles=\{\(paths\) => void appendVideosToPlaybackPool\(paths\)\}/);
  assert.match(app, /importBusy=\{importVideoBusy\}/);
  assert.match(panel, /追加后最多可保留 100 项/);
});

test('播放池按媒体类型展示元数据，音频不伪造视频分辨率', async () => {
  const panel = await readSource('./desktop/playback-pool-panel.tsx');

  assert.match(panel, /media_kind: 'video' \| 'audio'/);
  assert.match(panel, /source\.media_kind === 'audio'/);
  assert.match(panel, /audio_sample_rate_hz/);
  assert.match(panel, /audio_channel_count/);
  assert.match(panel, /PictureOutlined/);
  assert.match(panel, /AudioOutlined/);
});
