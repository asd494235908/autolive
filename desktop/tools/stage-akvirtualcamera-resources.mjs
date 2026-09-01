import { copyFile, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';

import { verifyAkVirtualCameraLock } from './verify-akvirtualcamera-lock.mjs';

const toolRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const repositoryRoot = resolve(toolRoot, '..');
const defaultResourceRoot = join(toolRoot, 'src-tauri', 'akvirtualcamera');

const ARTIFACT_DESTINATIONS = Object.freeze({
  'directshow-filter:x86': 'x86/AkVirtualCamera.dll',
  'directshow-filter:x64': 'x64/AkVirtualCamera.dll',
  'assistant:x64': 'x64/AkVCamAssistant.exe',
  'manager:x64': 'x64/AkVCamManager.exe',
  'capi:x64': 'bin/vcam_capi.dll',
  'sidecar:x64': 'bin/akvirtualcamera-sidecar-x64.exe',
});

const DOCUMENT_DESTINATIONS = Object.freeze([
  'COPYING',
  'MODIFICATIONS.md',
  'corresponding-source-manifest.json',
  'sbom.cdx.json',
]);

function artifactKey(artifact) {
  return `${artifact.role}:${artifact.architecture}`;
}

export function buildStagePlan(lock, resourceRoot = defaultResourceRoot, repoRoot = repositoryRoot) {
  const artifactByKey = new Map(
    lock.release_requirements.artifacts.map((artifact) => [artifactKey(artifact), artifact]),
  );
  const artifacts = Object.entries(ARTIFACT_DESTINATIONS).map(([key, relativeDestination]) => {
    const declaration = artifactByKey.get(key);
    if (!declaration) throw new Error(`锁文件缺少 AkVirtualCamera 产物声明：${key}`);
    return {
      source: resolve(repoRoot, declaration.path),
      destination: join(resourceRoot, ...relativeDestination.split('/')),
      relativeDestination,
    };
  });
  const documentByName = new Map(
    lock.release_requirements.documents.map((document) => [document.path.split('/').at(-1), document]),
  );
  const documents = DOCUMENT_DESTINATIONS.map((name) => {
    const declaration = documentByName.get(name);
    if (!declaration) throw new Error(`锁文件缺少 AkVirtualCamera 文档声明：${name}`);
    return {
      source: resolve(repoRoot, declaration.path),
      destination: join(resourceRoot, name),
      relativeDestination: name,
    };
  });
  return { artifacts, documents };
}

export async function stageAkVirtualCameraResources({
  repoRoot = repositoryRoot,
  resourceRoot = join(repoRoot, 'desktop', 'src-tauri', 'akvirtualcamera'),
} = {}) {
  // 该标记是运行时把资源视为可安装的唯一开关。每次正式 staging 先撤销旧标记，
  // 即使本次校验或复制失败也不会继续暴露上一轮发布状态。
  const marker = join(resourceRoot, 'release-ready.json');
  await rm(marker, { force: true });
  const lockFile = await readFile(join(repoRoot, 'desktop', 'third_party', 'akvirtualcamera', 'upstream.lock.json'), 'utf8');
  const lock = JSON.parse(lockFile);
  const report = await verifyAkVirtualCameraLock(lock, { repoRoot });
  if (!report.releaseReady) {
    throw new Error(`AkVirtualCamera 发布门禁未通过：${report.blockers.join('；')}`);
  }
  const plan = buildStagePlan(lock, resourceRoot, repoRoot);
  for (const file of [...plan.artifacts, ...plan.documents]) {
    await mkdir(dirname(file.destination), { recursive: true });
    await copyFile(file.source, file.destination);
  }
  await writeFile(
    marker,
    `${JSON.stringify({
      schemaVersion: 1,
      component: 'akvirtualcamera',
      releaseReady: true,
      commit: report.commit,
      licenseExpression: report.licenseExpression,
      runtime: lock.runtime,
      stagedFiles: [...plan.artifacts, ...plan.documents].map(({ relativeDestination }) => relativeDestination),
    }, null, 2)}\n`,
    'utf8',
  );
  return { report, plan, marker };
}

async function main() {
  const result = await stageAkVirtualCameraResources();
  console.log(JSON.stringify({
    releaseReady: result.report.releaseReady,
    marker: result.marker,
    files: result.plan.artifacts.length + result.plan.documents.length,
  }));
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 2;
  });
}
