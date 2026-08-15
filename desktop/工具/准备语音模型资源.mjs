import { execFileSync, spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, readdirSync, rmSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const supportedTargets = new Set([
  'x86_64-apple-darwin',
  'aarch64-apple-darwin',
  'x86_64-pc-windows-msvc',
]);

export function resolveTargetTriple(value = process.env.AUTOLIVE_TARGET_TRIPLE) {
  const target =
    value ??
    (process.platform === 'darwin' && process.arch === 'x64'
      ? 'x86_64-apple-darwin'
      : process.platform === 'darwin' && process.arch === 'arm64'
        ? 'aarch64-apple-darwin'
        : process.platform === 'win32' && process.arch === 'x64'
          ? 'x86_64-pc-windows-msvc'
          : null);
  if (!target || !supportedTargets.has(target)) {
    throw new Error(`当前构建平台不受支持：${target ?? `${process.platform}/${process.arch}`}`);
  }
  return target;
}

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
  throw new Error('找不到可用的 Python，请设置 AUTOLIVE_VOICE_PYTHON 指向已安装 voice clone 依赖的 Python 3.11。');
}

export function modelResourceLayout(root) {
  return {
    demucs: join(root, 'huggingface', 'hub', 'models--adefossez--HTDemucs'),
    whisper: join(root, 'huggingface', 'hub', 'models--Systran--faster-whisper-small'),
    xtts: join(root, 'tts', 'tts', 'tts_models--multilingual--multi-dataset--xtts_v2'),
  };
}

export function modelEnvironment(root) {
  const modelRoot = resolve(root);
  const huggingfaceRoot = join(modelRoot, 'huggingface');
  return {
    ...process.env,
    AUTOLIVE_VOICE_CLONE_MODEL_ROOT: modelRoot,
    TORCH_HOME: join(modelRoot, 'torch'),
    HF_HOME: huggingfaceRoot,
    HF_HUB_CACHE: join(huggingfaceRoot, 'hub'),
    TTS_HOME: join(modelRoot, 'tts'),
    PYTHONUNBUFFERED: '1',
  };
}

function hasNonEmptyFile(path) {
  try {
    const metadata = statSync(path);
    return metadata.isFile() && metadata.size > 0;
  } catch {
    return false;
  }
}

function snapshotDirectories(directory) {
  try {
    return readdirSync(join(directory, 'snapshots'), { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => join(directory, 'snapshots', entry.name));
  } catch {
    return [];
  }
}

function hasSnapshotFiles(directory, requiredFiles, extension) {
  if (!hasNonEmptyFile(join(directory, 'refs', 'main'))) return false;
  return snapshotDirectories(directory).some((snapshot) => {
    if (!requiredFiles.every((file) => hasNonEmptyFile(join(snapshot, file)))) return false;
    if (!extension) return true;
    try {
      return readdirSync(snapshot, { withFileTypes: true }).some(
        (entry) => entry.name.endsWith(extension) && hasNonEmptyFile(join(snapshot, entry.name)),
      );
    } catch {
      return false;
    }
  });
}

function hasXttsFiles(directory) {
  return [
    'config.json',
    'model.pth',
    'speakers_xtts.pth',
    'vocab.json',
  ].every((file) => hasNonEmptyFile(join(directory, file)));
}

function ensureCoquiTermsAccepted(cacheRoot) {
  const xttsRoot = modelResourceLayout(cacheRoot).xtts;
  if (hasXttsFiles(xttsRoot)) return;
  if (process.env.AUTOLIVE_COQUI_TOS_AGREE?.trim().toLowerCase() !== 'yes') {
    throw new Error(
      '首次下载 XTTS-v2 前需要确认 Coqui 许可条款。确认后请设置 AUTOLIVE_COQUI_TOS_AGREE=yes 再重试。',
    );
  }
  rmSync(xttsRoot, { recursive: true, force: true });
}

export function assertModelCacheComplete(root) {
  const layout = modelResourceLayout(root);
  const complete = {
    demucs: hasSnapshotFiles(layout.demucs, ['htdemucs.yaml'], '.safetensors'),
    whisper: hasSnapshotFiles(layout.whisper, ['config.json', 'model.bin', 'tokenizer.json', 'vocabulary.txt']),
    xtts: hasXttsFiles(layout.xtts),
  };
  const missing = Object.entries(layout)
    .filter(([name]) => !complete[name])
    .map(([name, directory]) => `${name}: ${directory}`);
  if (missing.length > 0) {
    throw new Error(`模型缓存不完整，请确认三套模型均已成功下载：\n${missing.join('\n')}`);
  }
  return layout;
}

export function isModelCacheComplete(root) {
  try {
    assertModelCacheComplete(root);
    return true;
  } catch {
    return false;
  }
}

export function assertPreparedVoiceModelResources(
  outputRoot = resolve(
    process.env.AUTOLIVE_VOICE_MODEL_OUTPUT_DIR ?? join(desktopRoot, 'src-tauri', 'voice-models'),
  ),
) {
  const resolvedOutputRoot = resolve(outputRoot);
  return {
    outputRoot: resolvedOutputRoot,
    layout: assertModelCacheComplete(resolvedOutputRoot),
  };
}

const MODEL_PROBE = String.raw`
from demucs.pretrained import get_model
get_model("htdemucs")
from faster_whisper import WhisperModel
WhisperModel("small", device="cpu", compute_type="int8")
from TTS.api import TTS
TTS(model_name="tts_models/multilingual/multi-dataset/xtts_v2", gpu=False)
print("voice clone models are ready")
`;

export function prepareVoiceModelResources({
  python = resolvePython(),
  cacheRoot = resolve(process.env.AUTOLIVE_VOICE_MODEL_CACHE_DIR ?? join(desktopRoot, 'src-tauri', 'target', 'voice-model-cache')),
  outputRoot = resolve(process.env.AUTOLIVE_VOICE_MODEL_OUTPUT_DIR ?? join(desktopRoot, 'src-tauri', 'voice-models')),
} = {}) {
  mkdirSync(cacheRoot, { recursive: true });
  ensureCoquiTermsAccepted(cacheRoot);
  const environment = modelEnvironment(cacheRoot);
  const probe = spawnSync(python, ['-c', MODEL_PROBE], {
    env: environment,
    input: process.env.AUTOLIVE_COQUI_TOS_AGREE?.trim().toLowerCase() === 'yes' ? 'y\n' : undefined,
    stdio: ['pipe', 'inherit', 'inherit'],
  });
  if (probe.error) {
    throw new Error(`模型下载 Python 启动失败：${probe.error.message}`);
  }
  if (probe.status !== 0) {
    throw new Error(`模型下载失败，Python 退出码：${probe.status ?? 'unknown'}`);
  }

  assertModelCacheComplete(cacheRoot);
  rmSync(outputRoot, { recursive: true, force: true });
  mkdirSync(dirname(outputRoot), { recursive: true });
  cpSync(cacheRoot, outputRoot, { recursive: true, force: true, dereference: true });
  assertModelCacheComplete(outputRoot);
  return { cacheRoot, outputRoot, layout: modelResourceLayout(outputRoot) };
}

function main() {
  const target = resolveTargetTriple();
  if (process.argv.includes('--assert-prepared')) {
    const result = assertPreparedVoiceModelResources();
    console.log(`已校验预准备语音模型资源：${target} -> ${result.outputRoot}`);
    return;
  }
  const python = resolvePython();
  const result = prepareVoiceModelResources({ python });
  console.log(`已准备语音模型资源：${target} -> ${result.outputRoot}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
