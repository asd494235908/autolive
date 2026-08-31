import { createHash } from 'node:crypto';
import { readdir, readFile } from 'node:fs/promises';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { parseRustGpu83Mappings } from './mpv-phase3a-candidate-contract.mjs';

export const PINNED_MPV_SOURCE_REF = '7b8915bc1d';
export const PINNED_MPV_RENDER_HEADER =
  `https://github.com/mpv-player/mpv/blob/${PINNED_MPV_SOURCE_REF}/include/mpv/render.h`;
export const PINNED_PUBLIC_RENDER_BACKENDS = Object.freeze(['opengl', 'sw']);
export const PRODUCTION_SHADER_SHA256 =
  '5b426433152f898eb2174bb84893747767aa77f1eb2b310cd7db1f2c4f0a1b40';
const PRODUCTION_RUNTIME_OPTION_NAMES = Object.freeze([
  'al_runtime_epoch_start_seconds',
  'al_runtime_source_fps',
  'al_runtime_random_seed',
  'al_runtime_plan_hi',
  'al_runtime_plan_lo',
  'al_runtime_frame_inner_percent',
  'al_runtime_frame_inter_percent',
  'al_runtime_frame_probability_percent',
  'al_runtime_slice_length_seconds',
  'al_runtime_slice_interval_seconds',
  'al_runtime_pip_jitter_px',
  'al_runtime_local_blur_interval_seconds',
  'al_runtime_smoothing_enabled',
  'al_runtime_smoothing_seconds',
  'al_runtime_highlight_enabled',
  'al_runtime_highlight_interval_seconds',
  'al_runtime_async_rotation_enabled',
  'al_runtime_async_rotation_min_degrees',
  'al_runtime_async_rotation_max_degrees',
]);
export const HISTORY_FIELD_PATHS = Object.freeze([
  'advanced.slice_min_length_ms',
  'advanced.picture_in_picture_timeline_locked',
]);

const REQUIRED_RUNTIME_FILES = Object.freeze(['mpv.exe']);
const LIBMPV_ARTIFACT = /(?:^|\/)(?:mpv(?:-\d+)?\.dll|mpv\.lib|libmpv[^/]*\.(?:a|dll|lib)|(?:include\/)?mpv\/(?:client|render)\.h)$/i;
const LIBMPV_CARGO_DEPENDENCY = /(?:^\s*["']?[A-Za-z0-9_-]*mpv[A-Za-z0-9_-]*["']?\s*=|^\s*\[[^\]]*(?:dependencies|build-dependencies|dev-dependencies)\.[^\]]*mpv[^\]]*\]|\bpackage\s*=\s*["'][^"']*mpv[^"']*["'])/im;
const LIBMPV_DECLARATION = /(?:\blibmpv\b|\bmpv(?:-\d+)?\.dll\b|\bmpv\.lib\b|\bmpv_render_[A-Za-z0-9_]+\b)/i;
const DYNAMIC_LIBMPV_LOADING = /(?:(?:LoadLibrary|GetProcAddress|libloading)[\s\S]{0,500}(?:libmpv|mpv(?:-\d+)?\.dll)|(?:libmpv|mpv(?:-\d+)?\.dll)[\s\S]{0,500}(?:LoadLibrary|GetProcAddress|libloading))/i;

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function sorted(values) {
  return [...values].sort((left, right) => left.localeCompare(right, 'en'));
}

function sanitizeRustSource(source) {
  let output = '';
  let index = 0;
  let blockDepth = 0;
  const strings = new Map();
  const saveString = (content) => {
    const key = `__RUST_STRING_${strings.size}__`;
    strings.set(key, content);
    return `"${key}"${'\n'.repeat((content.match(/\n/g) ?? []).length)}`;
  };
  while (index < source.length) {
    const current = source[index];
    const next = source[index + 1];
    if (blockDepth > 0) {
      if (current === '/' && next === '*') {
        blockDepth += 1;
        output += '  ';
        index += 2;
      } else if (current === '*' && next === '/') {
        blockDepth -= 1;
        output += '  ';
        index += 2;
      } else {
        output += current === '\n' ? '\n' : ' ';
        index += 1;
      }
      continue;
    }
    const rawString = /^(?:br|r)(#*)"/.exec(source.slice(index));
    if (rawString) {
      const contentStart = index + rawString[0].length;
      const terminator = `"${rawString[1]}`;
      const contentEnd = source.indexOf(terminator, contentStart);
      assert(contentEnd >= 0, 'Rust capability 源码包含未闭合原始字符串');
      output += saveString(source.slice(contentStart, contentEnd));
      index = contentEnd + terminator.length;
      continue;
    }
    const quoteIndex = current === '"' ? index : (current === 'b' && next === '"' ? index + 1 : -1);
    if (quoteIndex >= 0) {
      let cursor = quoteIndex + 1;
      while (cursor < source.length && source[cursor] !== '"') {
        cursor += source[cursor] === '\\' ? 2 : 1;
      }
      assert(cursor < source.length, 'Rust capability 源码包含未闭合字符串');
      output += saveString(source.slice(quoteIndex + 1, cursor));
      index = cursor + 1;
    } else if (current === '/' && next === '/') {
      output += '  ';
      index += 2;
      while (index < source.length && source[index] !== '\n') {
        output += ' ';
        index += 1;
      }
    } else if (current === '/' && next === '*') {
      blockDepth = 1;
      output += '  ';
      index += 2;
    } else {
      output += current;
      index += 1;
    }
  }
  assert(blockDepth === 0, 'Rust capability 源码包含未闭合块注释');
  return { source: output, strings };
}

function restoreRustString(value, strings) {
  const restored = strings.get(value);
  assert(restored !== undefined, `Rust 字符串占位符无效：${value}`);
  return restored;
}

function parseCapabilityTable(rustSource) {
  const declaredCountMatch = String(rustSource).match(
    /pub const GPU83_PARAMETER_COUNT:\s*usize\s*=\s*([\d_]+)\s*;/,
  );
  const tableMatch = String(rustSource).match(
    /pub static GPU83_PARAMETER_MAPPINGS:[^=]+?=\s*\[([\s\S]*?)\r?\n\];/,
  );
  assert(declaredCountMatch && tableMatch, '无法解析 Rust GPU83 capability 映射表');
  const declaredCount = Number(declaredCountMatch[1].replaceAll('_', ''));
  const mappings = parseRustGpu83Mappings(tableMatch[1]);
  assert(mappings.length === declaredCount, `GPU83 映射数量漂移：声明=${declaredCount}，实际=${mappings.length}`);
  assert(new Set(mappings.map(({ fieldPath }) => fieldPath)).size === mappings.length,
    'GPU83 字段路径必须唯一');
  assert(new Set(mappings.map(({ shaderOption }) => shaderOption)).size === mappings.length,
    'GPU83 shader 选项必须唯一');
  return { declaredCount, mappings };
}

function verifyManifest(manifest) {
  assert(manifest?.schema_version === 1, 'mpv 运行清单 schema 必须固定为 1');
  assert(manifest?.components?.mpv?.source_ref === PINNED_MPV_SOURCE_REF,
    'mpv 固定源码引用漂移，必须重新完成 Phase 3C 技术审核');
  assert(manifest?.components?.mpv?.version === 'v0.41.0-923-g7b8915bc1',
    'mpv 固定版本漂移，必须重新完成 Phase 3C 技术审核');
  assert(manifest?.components?.libplacebo?.version === 'v7.371.0',
    'libplacebo 固定版本漂移，必须重新完成 Phase 3C 技术审核');
  assert(manifest?.components?.libplacebo?.source_ref === '22ee762',
    'libplacebo 固定源码引用漂移，必须重新完成 Phase 3C 技术审核');
  const manifestFiles = sorted(Object.keys(manifest?.files ?? {}));
  for (const required of REQUIRED_RUNTIME_FILES) {
    assert(manifestFiles.includes(required), `mpv 运行清单缺少必需文件：${required}`);
  }
  const unexpected = manifestFiles.filter((path) => LIBMPV_ARTIFACT.test(path.replaceAll('\\', '/')));
  assert(unexpected.length === 0, `mpv 运行清单出现未审核 libmpv 资产：${unexpected.join(', ')}`);
  return manifestFiles;
}

function verifyCargoBoundary(mainCargoSource, dependencyCargoSource, resourceDeclarationSource) {
  const lintHeader = /^\s*\[lints\.rust\]\s*(?:#.*)?$/m.exec(mainCargoSource);
  assert(lintHeader, '主 Tauri Cargo 缺少 [lints.rust]');
  const lintTail = mainCargoSource.slice(lintHeader.index + lintHeader[0].length);
  const nextSection = /^\s*\[/m.exec(lintTail);
  const lintBody = lintTail.slice(0, nextSection?.index ?? lintTail.length);
  assert(/^\s*unsafe_code\s*=\s*"forbid"\s*(?:#.*)?$/m.test(lintBody),
    'Rust unsafe_code 不再是 forbid，必须先完成 FFI 安全边界专项审核');
  assert(!LIBMPV_CARGO_DEPENDENCY.test(dependencyCargoSource),
    '检测到 libmpv 依赖，但 Phase 3C Render API/FFI 尚未完成准入审核');
  assert(!LIBMPV_DECLARATION.test(resourceDeclarationSource),
    '运行资源或构建脚本检测到未审核的 libmpv 声明');
}

function verifyRuntimeArtifacts(resourcePaths) {
  const normalized = resourcePaths.map((path) => String(path).replaceAll('\\', '/'));
  const unexpected = normalized.filter((path) => LIBMPV_ARTIFACT.test(path));
  assert(unexpected.length === 0,
    `检测到未审核的 libmpv 开发/运行资产：${unexpected.join(', ')}`);
  assert(normalized.some((path) => basename(path).toLowerCase() === 'mpv.exe'),
    '缺少当前外部 mpv.exe 运行资产');
}

function verifyExternalJsonIpcTransport(backendSource, rustImplementationSource) {
  const required = [
    ['--vo=gpu-next', '缺少 gpu-next 外部播放器参数'],
    ['--input-ipc-server=', '缺少外部 mpv JSON IPC 管道参数'],
    ['glsl-shader-opts', '缺少周期 shader 参数快照更新'],
    ['std::process::Command', '当前实现不再是受管外部 mpv 进程'],
  ];
  for (const [needle, message] of required) assert(backendSource.includes(needle), message);
  assert(!LIBMPV_DECLARATION.test(rustImplementationSource),
    '检测到未审核的 libmpv Render API 调用');
  assert(!DYNAMIC_LIBMPV_LOADING.test(rustImplementationSource),
    '检测到未审核的 libmpv 动态加载路径');
}

function shaderStateDirectives(productionShaderSource, candidateShaders) {
  const entries = [{ path: 'production/gpu83.hook', source: productionShaderSource }, ...candidateShaders];
  return entries.flatMap(({ path, source }) => [...String(source).matchAll(/^\/\/!(SAVE|BUFFER)\b.*$/gm)]
    .map((match) => ({ path, directive: match[1], line: match[0] })));
}

function macroBody(source, name) {
  const start = source.indexOf(`macro_rules! ${name}`);
  assert(start >= 0, `缺少 Rust 映射宏：${name}`);
  const boundaries = [
    source.indexOf('macro_rules!', start + 1),
    source.indexOf('\nconst AVAILABLE', start + 1),
  ].filter((index) => index >= 0);
  return source.slice(start, boundaries.length === 0 ? source.length : Math.min(...boundaries));
}

function verifyRustCapabilitySemantics(rustSource) {
  const constants = [
    [/const AVAILABLE:\s*Gpu83ParameterCapability\s*=\s*Gpu83ParameterCapability::ShaderParameter\s*;/s,
      'AVAILABLE 不再等于 ShaderParameter'],
    [/const UNVERIFIED:\s*Gpu83ParameterCapability\s*=\s*Gpu83ParameterCapability::Unavailable\(\s*ALGORITHM_NOT_VERIFIED\s*\)\s*;/s,
      'UNVERIFIED 不再严格 fail-closed'],
    [/const SCHEDULER:\s*Gpu83ParameterCapability\s*=\s*Gpu83ParameterCapability::ScheduledParameter\s*;/s,
      'SCHEDULER 不再由受限 PTS 调度器执行'],
    [/const HISTORY:\s*Gpu83ParameterCapability\s*=\s*Gpu83ParameterCapability::Unavailable\(\s*REQUIRES_HISTORY_TEXTURE\s*\)\s*;/s,
      'HISTORY capability 不再严格 fail-closed'],
  ];
  for (const [pattern, message] of constants) assert(pattern.test(rustSource), message);
  for (const name of ['video_value', 'video_flag', 'advanced_value', 'advanced_flag', 'advanced_optional']) {
    assert(/capability:\s*\$capability\b/.test(macroBody(rustSource, name)),
      `${name} 不再把 capability 参数写入真实映射`);
  }
  assert(/capability:\s*AVAILABLE\b/s.test(macroBody(rustSource, 'visual_band')),
    'visual_band 不再是已验证生产参数');
}

function verifyProductionIdentity(mappings, shaderSource) {
  const shaderOptions = [...String(shaderSource).matchAll(/^\/\/!PARAM\s+(al_[A-Za-z0-9_]+)\s*$/gm)]
    .map((match) => match[1]);
  const runtimeOptionSet = new Set(PRODUCTION_RUNTIME_OPTION_NAMES);
  const productShaderOptions = shaderOptions.filter((name) => !runtimeOptionSet.has(name));
  const runtimeShaderOptions = shaderOptions.filter((name) => runtimeOptionSet.has(name));
  const availableOptions = mappings
    .filter(({ capability }) => capability === 'AVAILABLE')
    .map(({ shaderOption }) => shaderOption);
  assert(JSON.stringify(sorted(availableOptions)) === JSON.stringify(sorted(productShaderOptions))
    && JSON.stringify(sorted(runtimeShaderOptions)) === JSON.stringify(sorted(PRODUCTION_RUNTIME_OPTION_NAMES)),
  `生产 shader 与 AVAILABLE 身份漂移：available=${availableOptions.join(',')}，product=${productShaderOptions.join(',')}，runtime=${runtimeShaderOptions.join(',')}`);
  const shaderSha256 = createHash('sha256').update(shaderSource, 'utf8').digest('hex');
  assert(shaderSha256 === PRODUCTION_SHADER_SHA256,
    `生产 shader 固定哈希漂移：${shaderSha256}`);
  return { shaderOptions: availableOptions, shaderSha256 };
}

export function buildPhase3cAdmissionReport({
  manifest,
  cargoSource,
  dependencyCargoSource = cargoSource,
  rustCapabilitySource,
  backendSource,
  rustImplementationSource = backendSource,
  resourceDeclarationSource = '',
  productionShaderSource,
  candidateShaders,
  resourcePaths,
}) {
  const manifestFiles = verifyManifest(manifest);
  verifyCargoBoundary(cargoSource, dependencyCargoSource, resourceDeclarationSource);
  verifyRuntimeArtifacts(resourcePaths);
  verifyExternalJsonIpcTransport(backendSource, rustImplementationSource);
  const sameFrameStateDirectives = shaderStateDirectives(productionShaderSource, candidateShaders);

  const sanitizedRust = sanitizeRustSource(rustCapabilitySource);
  const rustCapabilityCode = sanitizedRust.source;
  const historyReasonMatch = /^\s*const REQUIRES_HISTORY_TEXTURE:\s*&str\s*=\s*"([^"]+)"\s*;/m
    .exec(rustCapabilityCode);
  assert(historyReasonMatch
    && restoreRustString(historyReasonMatch[1], sanitizedRust.strings)
      === 'requires_history_or_secondary_texture', '历史纹理不可用原因常量漂移');
  verifyRustCapabilitySemantics(rustCapabilityCode);

  const parsedCapabilityTable = parseCapabilityTable(rustCapabilityCode);
  const declaredCount = parsedCapabilityTable.declaredCount;
  const mappings = parsedCapabilityTable.mappings.map((mapping) => ({
    ...mapping,
    fieldPath: restoreRustString(mapping.fieldPath, sanitizedRust.strings),
    shaderOption: restoreRustString(mapping.shaderOption, sanitizedRust.strings),
  }));
  const historyMappings = mappings.filter(({ capability }) => capability === 'HISTORY');
  const timelineLock = mappings.find(
    ({ fieldPath }) => fieldPath === 'advanced.picture_in_picture_timeline_locked',
  );
  assert(timelineLock?.capability === 'HISTORY',
    '画中画时间轴锁定字段不得绕过跨帧组合门禁');
  assert(JSON.stringify(historyMappings.map(({ fieldPath }) => fieldPath)) === JSON.stringify(HISTORY_FIELD_PATHS),
    `历史纹理字段集合或顺序漂移：${historyMappings.map(({ fieldPath }) => fieldPath).join(', ')}`);
  const productionIdentity = verifyProductionIdentity(mappings, productionShaderSource);
  const shaderCount = mappings.filter(({ capability }) => capability === 'AVAILABLE').length;
  const schedulerCount = mappings.filter(({ capability }) => capability === 'SCHEDULER').length;
  const unverifiedCount = mappings.filter(({ capability }) => capability === 'UNVERIFIED').length;
  const executableCount = shaderCount + schedulerCount;
  assert(declaredCount === 83 && shaderCount === 61 && schedulerCount === 18 && unverifiedCount === 2,
    `开发执行 capability 必须保持 61 shader + 18 scheduler + 2 unverified color /83，当前为 ${shaderCount}+${schedulerCount}+${unverifiedCount}/${declaredCount}`);

  return {
    schemaVersion: 1,
    gate: 'mpv-libplacebo-phase3c-admission-audit',
    status: 'not_admitted',
    checkStatus: 'passed',
    admissionStatus: 'not_admitted',
    phase3cImplemented: false,
    productionCapability: `${executableCount}/${declaredCount}`,
    shaderCapability: `${shaderCount}/${declaredCount}`,
    schedulerCapability: `${schedulerCount}/${declaredCount}`,
    unverifiedColorCapability: `${unverifiedCount}/${declaredCount}`,
    currentTransport: 'external_mpv_json_ipc',
    historyPromotionAllowed: false,
    historyCapability: {
      status: 'not_admitted',
      reason: 'requires_history_or_secondary_texture',
      fieldCount: historyMappings.length,
      fields: historyMappings.map(({ fieldPath }) => fieldPath),
    },
    crossFrameCombinationGuard: {
      status: 'not_admitted',
      field: 'advanced.picture_in_picture_timeline_locked',
      currentCapability: timelineLock.capability,
      rule: '时间轴解锁不得仅因调度器可用而放行；还必须具备独立源时间轴与有界历史纹理证据',
    },
    runtime: {
      artifactKind: 'executable_only',
      manifestFiles,
      libmpvArtifactsPresent: false,
      rustLibmpvDependencyPresent: false,
      unsafeCodePolicy: 'forbid',
    },
    productionIdentity: {
      availableShaderOptions: productionIdentity.shaderOptions,
      productionShaderSha256: productionIdentity.shaderSha256,
    },
    sameFrameShaderState: {
      directiveCount: sameFrameStateDirectives.length,
      directives: sameFrameStateDirectives,
      admissionMeaning: 'none',
    },
    reviewedPinnedSourceContract: {
      mpvSourceRef: PINNED_MPV_SOURCE_REF,
      source: PINNED_MPV_RENDER_HEADER,
      verification: 'external_primary_source_review_not_local_header_validation',
      publicBackends: PINNED_PUBLIC_RENDER_BACKENDS,
      d3d11PublicRenderBackend: false,
      vulkanPublicRenderBackend: false,
    },
    blockers: [
      '当前资产只有外部 mpv.exe，不含 libmpv DLL、导入库或开发头文件',
      '固定 mpv 源码引用的公开 Render API 只有 OpenGL 与软件后端，未提供 D3D11/Vulkan 公共 Render API',
      'Rust 工程禁止 unsafe 且没有经审核的 libmpv FFI 封装',
      '当前外部 JSON IPC 与同帧 shader 没有有界跨帧纹理环、取消和设备丢失所有权',
    ],
    requiredBeforeAdmission: [
      '选定并固定提供 D3D11/Vulkan 生产渲染面的成熟上游接口与可再分发资产',
      '完成最小 FFI 安全封装、线程所有权、取消、超时、设备丢失和资源释放设计',
      '实现有界纹理环与显存上限，并证明读取的是真实历史帧而非同帧中间纹理',
      '通过一次性授权的 1080p60 D3D11/Vulkan 实机历史差异、P99、长稳和恢复门禁',
    ],
  };
}

async function listFiles(root) {
  const entries = await readdir(root, { withFileTypes: true });
  const nested = await Promise.all(entries.map(async (entry) => {
    const path = join(root, entry.name);
    return entry.isDirectory() ? listFiles(path) : [path];
  }));
  return nested.flat();
}

async function listMpvThirdPartyFiles(thirdPartyRoot) {
  const entries = await readdir(thirdPartyRoot, { withFileTypes: true });
  const relevant = entries.filter((entry) => /mpv/i.test(entry.name));
  const nested = await Promise.all(relevant.map((entry) => {
    const path = join(thirdPartyRoot, entry.name);
    return entry.isDirectory() ? listFiles(path) : [path];
  }));
  return nested.flat();
}

export async function runPhase3cAdmissionGate() {
  const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
  const tauriRoot = join(desktopRoot, 'src-tauri');
  const cratesRoot = join(desktopRoot, 'crates');
  const candidateRoot = join(desktopRoot, 'tools', 'shader-candidates');
  const candidatePaths = (await listFiles(candidateRoot)).filter((path) => path.endsWith('.hook'));
  const crateCargoPaths = (await listFiles(cratesRoot)).filter((path) => basename(path) === 'Cargo.toml');
  const rustPaths = [
    ...(await listFiles(join(tauriRoot, 'src'))),
    ...(await listFiles(cratesRoot)),
  ].filter((path) => path.endsWith('.rs'));
  const [manifestSource, cargoSource, rustCapabilitySource, backendSource, productionShaderSource,
    candidateSources, thirdPartyPaths, binaryPaths, allCargoSources, allRustSources,
    runtimeResourcesSource, tauriConfigSource, prepareResourcesSource, buildSource] = await Promise.all([
    readFile(join(desktopRoot, 'third_party', 'mpv', 'x86_64-pc-windows-msvc', 'legal',
      'mpv-runtime-manifest.json'), 'utf8'),
    readFile(join(tauriRoot, 'Cargo.toml'), 'utf8'),
    readFile(join(tauriRoot, 'src', 'media_video_gpu_effects.rs'), 'utf8'),
    readFile(join(tauriRoot, 'src', 'realtime_video_backend.rs'), 'utf8'),
    readFile(join(tauriRoot, 'resources', 'shaders', 'gpu83.hook'), 'utf8'),
    Promise.all(candidatePaths.map(async (path) => ({ path, source: await readFile(path, 'utf8') }))),
    listMpvThirdPartyFiles(join(desktopRoot, 'third_party')),
    listFiles(join(tauriRoot, 'binaries')),
    Promise.all(crateCargoPaths.map((path) => readFile(path, 'utf8'))),
    Promise.all(rustPaths.map((path) => readFile(path, 'utf8'))),
    readFile(join(tauriRoot, 'runtime-resources.json'), 'utf8'),
    readFile(join(tauriRoot, 'tauri.conf.json'), 'utf8'),
    readFile(join(desktopRoot, 'tools', 'prepare-ffmpeg-resources.mjs'), 'utf8'),
    readFile(join(tauriRoot, 'build.rs'), 'utf8'),
  ]);
  return buildPhase3cAdmissionReport({
    manifest: JSON.parse(manifestSource),
    cargoSource,
    dependencyCargoSource: `${cargoSource}\n${allCargoSources.join('\n')}`,
    rustCapabilitySource,
    backendSource,
    rustImplementationSource: allRustSources.join('\n'),
    resourceDeclarationSource: [runtimeResourcesSource, tauriConfigSource,
      prepareResourcesSource, buildSource].join('\n'),
    productionShaderSource,
    candidateShaders: candidateSources,
    resourcePaths: [...thirdPartyPaths, ...binaryPaths],
  });
}

async function main() {
  try {
    process.stdout.write(`${JSON.stringify(await runPhase3cAdmissionGate(), null, 2)}\n`);
  } catch (error) {
    process.stderr.write(`${JSON.stringify({
      schemaVersion: 1,
      gate: 'mpv-libplacebo-phase3c-admission-audit',
      status: 'failed',
      checkStatus: 'failed',
      admissionStatus: 'not_admitted',
      phase3cImplemented: false,
      historyPromotionAllowed: false,
      error: error.message,
    }, null, 2)}\n`);
    process.exitCode = 1;
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) await main();
