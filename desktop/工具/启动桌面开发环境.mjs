import { spawn, spawnSync } from 'node:child_process';
import { dirname, delimiter, join, resolve } from 'node:path';
import { statSync } from 'node:fs';
import { homedir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { isModelCacheComplete } from './准备语音模型资源.mjs';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const repositoryRoot = dirname(desktopRoot);

function isExecutable(path) {
  try {
    const metadata = statSync(path);
    if (!metadata.isFile()) return false;
    if (process.platform === 'win32') return true;
    return (metadata.mode & 0o111) !== 0;
  } catch {
    return false;
  }
}

function pythonHasVoiceCloneDependencies(python) {
  const probe = spawnSync(
    python,
    [
      '-c',
      "import importlib.util; import sys; sys.exit(0 if all(importlib.util.find_spec(name) for name in ('demucs', 'faster_whisper', 'TTS')) else 1)",
    ],
    { stdio: 'ignore' },
  );
  return probe.status === 0;
}

function pythonCandidates(root, environment) {
  const explicit = environment.AUTOLIVE_VOICE_PYTHON?.trim();
  if (explicit) return [resolve(explicit)];

  const isWindows = process.platform === 'win32';
  const executable = isWindows ? join('Scripts', 'python.exe') : join('bin', 'python');
  const candidates = [
    join(repositoryRoot, '.venv-voice-clone', executable),
    join(homedir(), 'miniconda3', 'envs', 'autolive-voice', executable),
    join(homedir(), 'mambaforge', 'envs', 'autolive-voice', executable),
    join(homedir(), 'anaconda3', 'envs', 'autolive-voice', executable),
  ];
  const pathEntries = (environment.PATH ?? '').split(delimiter).filter(Boolean);
  for (const entry of pathEntries) {
    candidates.push(join(entry, isWindows ? 'python.exe' : 'python3'));
    candidates.push(join(entry, isWindows ? 'python.exe' : 'python'));
  }
  return candidates;
}

export function resolveDevEnvironment({ root = desktopRoot, env = process.env } = {}) {
  const environment = { ...env };
  const workerPath = join(root, 'worker', 'voice_clone_adapter.py');
  if (!environment.AUTOLIVE_VOICE_CLONE_WORKER && isExecutable(workerPath)) {
    environment.AUTOLIVE_VOICE_CLONE_WORKER = workerPath;
  }

  const extension = process.platform === 'win32' ? '.exe' : '';
  const ffmpegPath = join(root, 'src-tauri', 'binaries', `ffmpeg${extension}`);
  const ffprobePath = join(root, 'src-tauri', 'binaries', `ffprobe${extension}`);
  if (!environment.AUTOLIVE_FFMPEG_PATH && isExecutable(ffmpegPath)) {
    environment.AUTOLIVE_FFMPEG_PATH = ffmpegPath;
  }
  if (!environment.AUTOLIVE_FFPROBE_PATH && isExecutable(ffprobePath)) {
    environment.AUTOLIVE_FFPROBE_PATH = ffprobePath;
  }

  const explicitPython = environment.AUTOLIVE_VOICE_PYTHON?.trim();
  const python = pythonCandidates(root, environment).find(
    (candidate) => isExecutable(candidate) && pythonHasVoiceCloneDependencies(candidate),
  );
  if (explicitPython && !python) {
    throw new Error(`AUTOLIVE_VOICE_PYTHON 不包含 Demucs、Whisper 和 TTS 依赖：${explicitPython}`);
  }
  if (python) {
    environment.AUTOLIVE_VOICE_PYTHON = python;
    const pythonDirectory = dirname(python);
    const pathEntries = (environment.PATH ?? '').split(delimiter).filter(Boolean);
    if (!pathEntries.includes(pythonDirectory)) {
      environment.PATH = [pythonDirectory, ...pathEntries].join(delimiter);
    }
  }

  const modelCacheRoot = resolve(
    environment.AUTOLIVE_VOICE_MODEL_CACHE_DIR?.trim() ||
      join(root, 'src-tauri', 'target', 'voice-model-cache'),
  );
  if (!environment.AUTOLIVE_VOICE_CLONE_MODEL_ROOT && isModelCacheComplete(modelCacheRoot)) {
    environment.AUTOLIVE_VOICE_CLONE_MODEL_ROOT = modelCacheRoot;
  }
  return environment;
}

function main() {
  const command = process.platform === 'win32' ? 'tauri.cmd' : 'tauri';
  const child = spawn(command, ['dev', '--config', 'src-tauri/tauri.conf.json'], {
    cwd: desktopRoot,
    env: resolveDevEnvironment(),
    stdio: 'inherit',
  });
  const forwardSignal = (signal) => {
    if (!child.killed) child.kill(signal);
  };
  process.once('SIGINT', () => forwardSignal('SIGINT'));
  process.once('SIGTERM', () => forwardSignal('SIGTERM'));
  child.once('error', (error) => {
    console.error(`启动 Tauri 开发版失败：${error.message}`);
    process.exitCode = 1;
  });
  child.once('exit', (code, signal) => {
    if (signal) return;
    process.exitCode = code ?? 1;
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
