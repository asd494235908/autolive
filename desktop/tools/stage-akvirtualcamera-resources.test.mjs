import test from 'node:test';
import { spawnSync } from 'node:child_process';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';

import { buildStagePlan, stageAkVirtualCameraResources } from './stage-akvirtualcamera-resources.mjs';

const toolsRoot = resolve(dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = resolve(toolsRoot, '..', '..');
const stageScriptPath = join(toolsRoot, 'stage-akvirtualcamera-resources.mjs');

test('AkVirtualCamera 资源暂存计划覆盖运行时文件且不把安装器放入资源树', async () => {
  const lock = JSON.parse(
    await readFile(join(repositoryRoot, 'desktop', 'third_party', 'akvirtualcamera', 'upstream.lock.json'), 'utf8'),
  );
  const plan = buildStagePlan(lock, 'resource-root', repositoryRoot);

  assert.deepEqual(
    plan.artifacts.map(({ relativeDestination }) => relativeDestination),
    [
      'x86/AkVirtualCamera.dll',
      'x64/AkVirtualCamera.dll',
      'x64/AkVCamAssistant.exe',
      'x64/AkVCamManager.exe',
      'bin/vcam_capi.dll',
      'bin/akvirtualcamera-sidecar-x64.exe',
    ],
  );
  assert.deepEqual(
    plan.documents.map(({ relativeDestination }) => relativeDestination),
    ['COPYING', 'MODIFICATIONS.md', 'corresponding-source-manifest.json', 'sbom.cdx.json'],
  );
  assert.equal(plan.artifacts.some(({ relativeDestination }) => relativeDestination.includes('installer')), false);
  assert.ok(plan.artifacts.every(({ source }) => source.startsWith(repositoryRoot)));
});

test('正式构建资源暂存在产物或签名缺失时以退出码 2 fail-closed', () => {
  const result = spawnSync(process.execPath, [stageScriptPath], {
    cwd: repositoryRoot,
    encoding: 'utf8',
  });

  assert.equal(result.status, 2);
  assert.match(result.stderr, /AkVirtualCamera 发布门禁未通过/);
  assert.doesNotMatch(result.stderr, /release-ready\.json 已生成/);
});

test('资源校验失败会先撤销旧 release-ready 标记', async () => {
  const resourceRoot = mkdtempSync(join(tmpdir(), 'autolive-akvcam-stage-'));
  const marker = join(resourceRoot, 'release-ready.json');
  writeFileSync(marker, '{"releaseReady":true}\n');

  await assert.rejects(
    () => stageAkVirtualCameraResources({ repoRoot: repositoryRoot, resourceRoot }),
    /AkVirtualCamera 发布门禁未通过/,
  );
  assert.equal(existsSync(marker), false);
});
