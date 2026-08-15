import { execFileSync, spawnSync } from 'node:child_process';
import { chmodSync, cpSync, mkdirSync, rmSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const workerSource = join(desktopRoot, 'worker', 'voice_clone_adapter.py');
const binaryBaseName = 'autolive-voice-clone-worker';

function isFile(path) {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function findExecutable(command) {
  const lookupCommand = process.platform === 'win32' ? 'where' : 'which';
  try {
    return execFileSync(lookupCommand, [command], { encoding: 'utf8' })
      .split(/\r?\n/)
      .find((path) => path.trim() && isFile(path.trim()))
      ?.trim();
  } catch {
    return undefined;
  }
}

export function workerBinaryName(target = process.env.AUTOLIVE_TARGET_TRIPLE) {
  return target?.includes('windows') || (!target && process.platform === 'win32')
    ? `${binaryBaseName}.exe`
    : binaryBaseName;
}

export function workerResourceLayout(
  root = resolve(process.env.AUTOLIVE_VOICE_WORKER_BUILD_DIR ?? join(desktopRoot, 'src-tauri', 'target', 'voice-worker-build')),
  target = process.env.AUTOLIVE_TARGET_TRIPLE,
) {
  const buildRoot = resolve(root);
  return {
    buildRoot,
    distRoot: join(buildRoot, 'dist'),
    workRoot: join(buildRoot, 'work'),
    specRoot: join(buildRoot, 'spec'),
    outputRoot: resolve(
      process.env.AUTOLIVE_VOICE_WORKER_OUTPUT_DIR ?? join(desktopRoot, 'src-tauri', 'voice-worker'),
    ),
    binaryName: workerBinaryName(target),
  };
}

export function resolveWorkerBinaryPath(outputRoot, target = process.env.AUTOLIVE_TARGET_TRIPLE) {
  return join(resolve(outputRoot), workerBinaryName(target));
}

export function resolvePython(explicit = process.env.AUTOLIVE_VOICE_PYTHON) {
  if (explicit?.trim()) {
    const command = explicit.trim();
    const path = resolve(command);
    if (isFile(path)) return path;
    if (!/[\\/]/.test(command)) {
      const executable = findExecutable(command);
      if (executable) return executable;
    }
    throw new Error(`找不到可用的 Python：${path}。请设置 AUTOLIVE_VOICE_PYTHON。`);
  }

  for (const command of ['python3', 'python']) {
    const path = findExecutable(command);
    if (path) return path;
  }
  throw new Error('找不到可用的 Python，请设置 AUTOLIVE_VOICE_PYTHON。');
}

export function prepareVoiceWorkerResource({
  python = resolvePython(),
  source = workerSource,
  target = process.env.AUTOLIVE_TARGET_TRIPLE,
  buildRoot,
  outputRoot,
} = {}) {
  if (!isFile(source)) {
    throw new Error(`找不到固定话术 Worker 源文件：${source}`);
  }
  const layout = workerResourceLayout(buildRoot, target);
  if (outputRoot) layout.outputRoot = resolve(outputRoot);
  mkdirSync(layout.buildRoot, { recursive: true });
  rmSync(layout.distRoot, { recursive: true, force: true });
  rmSync(layout.workRoot, { recursive: true, force: true });
  rmSync(layout.specRoot, { recursive: true, force: true });

  const result = spawnSync(
    python,
    [
      '-m',
      'PyInstaller',
      '--noconfirm',
      '--clean',
      '--onefile',
      '--name',
      binaryBaseName,
      '--distpath',
      layout.distRoot,
      '--workpath',
      layout.workRoot,
      '--specpath',
      layout.specRoot,
      '--collect-all',
      'demucs',
      '--collect-all',
      'faster_whisper',
      '--collect-all',
      'TTS',
      '--collect-all',
      'torch',
      '--collect-all',
      'torchaudio',
      '--hidden-import',
      'demucs.separate',
      '--hidden-import',
      'faster_whisper',
      '--hidden-import',
      'TTS.api',
      source,
    ],
    { stdio: ['ignore', 'inherit', 'inherit'] },
  );
  if (result.error) {
    throw new Error(`固定话术 Worker 打包启动失败：${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`固定话术 Worker 打包失败，Python 退出码：${result.status ?? 'unknown'}`);
  }

  const builtBinary = join(layout.distRoot, layout.binaryName);
  if (!isFile(builtBinary)) {
    throw new Error(`固定话术 Worker 打包完成但未找到产物：${builtBinary}`);
  }
  rmSync(layout.outputRoot, { recursive: true, force: true });
  mkdirSync(dirname(layout.outputRoot), { recursive: true });
  mkdirSync(layout.outputRoot, { recursive: true });
  const outputBinary = resolveWorkerBinaryPath(layout.outputRoot, target);
  cpSync(builtBinary, outputBinary, { force: true });
  if (process.platform !== 'win32') chmodSync(outputBinary, 0o755);
  return { ...layout, outputBinary };
}

function main() {
  const result = prepareVoiceWorkerResource();
  console.log(`已准备固定话术 Worker：${result.outputBinary}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
