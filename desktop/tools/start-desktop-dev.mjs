import { spawn } from 'node:child_process';
import { join, resolve } from 'node:path';
import { statSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));

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

export function resolveDevEnvironment({ root = desktopRoot, env = process.env } = {}) {
  const environment = { ...env };
  environment.VITE_CONTROL_PLANE_BASE_URL ||= 'http://101.96.208.132:9090';
  environment.VITE_CONTROL_PLANE_ENV ||= 'test';
  const extension = process.platform === 'win32' ? '.exe' : '';
  const ffmpegPath = join(root, 'src-tauri', 'binaries', `ffmpeg${extension}`);
  const ffprobePath = join(root, 'src-tauri', 'binaries', `ffprobe${extension}`);
  if (!environment.AUTOLIVE_FFMPEG_PATH && isExecutable(ffmpegPath)) {
    environment.AUTOLIVE_FFMPEG_PATH = ffmpegPath;
  }
  if (!environment.AUTOLIVE_FFPROBE_PATH && isExecutable(ffprobePath)) {
    environment.AUTOLIVE_FFPROBE_PATH = ffprobePath;
  }

  return environment;
}

function main() {
  const environment = resolveDevEnvironment();
  const command = process.platform === 'win32' ? 'tauri.cmd' : 'tauri';
  const tauriConfig = environment.AUTOLIVE_TAURI_CONFIG || 'src-tauri/tauri.test.conf.json';
  const child = spawn(command, ['dev', '--config', tauriConfig], {
    cwd: desktopRoot,
    env: environment,
    stdio: 'inherit',
    shell: process.platform === 'win32',
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
