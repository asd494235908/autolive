import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const lockPath = fileURLToPath(
  new URL('../third_party/mpv/reproducible-build-lock.json', import.meta.url),
);

const SHA256 = /^[a-f0-9]{64}$/;
const COMMIT = /^[a-f0-9]{40}$/;
const REQUIRED_SOURCES = Object.freeze([
  'fast-float', 'ffmpeg', 'freetype', 'fribidi', 'glslang', 'harfbuzz', 'jinja',
  'lcms2', 'libass', 'libplacebo', 'markupsafe', 'mpv', 'shaderc', 'spirv-cross',
  'spirv-headers', 'spirv-tools', 'vulkan-headers', 'vulkan-loader', 'zstd',
]);
const REQUIRED_FEATURES = Object.freeze([
  'cplayer', 'd3d11', 'd3d-hwaccel', 'lcms2', 'shaderc', 'spirv-cross', 'vulkan',
  'win32-threads',
]);
const DISABLED_FEATURES = Object.freeze([
  'cdda', 'cplugins', 'dvbin', 'dvdnav', 'gl', 'javascript', 'libarchive', 'libavdevice',
  'libbluray', 'libcurl', 'libmpv', 'lua', 'manpage-build', 'openal', 'sdl2-audio',
  'sdl2-gamepad', 'sdl2-video', 'sixel', 'subrandr', 'vapoursynth', 'wasapi',
]);
const DISABLED_BOOLEAN_OPTIONS = Object.freeze(['fuzzers', 'tests']);
const REQUIRED_TOOLS = Object.freeze([
  'clang', 'cmake', 'lld-link', 'llvm-rc', 'meson', 'nasm', 'ninja', 'pkg-config',
  'python', 'ucrt', 'windows-sdk', 'msvc-toolset',
]);
const REQUIRED_OUTPUT_ROLES = Object.freeze([
  'build-log', 'build-parameters', 'corresponding-source-archive',
  'copyright-inventory', 'cyclonedx-sbom', 'dependency-lock', 'docker-host-evidence',
  'license-inventory', 'meson-introspection', 'mpv-executable', 'patch-bundle',
  'pe-imports', 'spdx-sbom', 'spirv-cross-runtime', 'vulkan-loader-runtime',
]);
const REQUIRED_PATCHES = Object.freeze([
  ['fribidi', 0, 'desktop/third_party/mpv/build/patches/fribidi-native-compiler.patch'],
  ['libass', 1, 'desktop/third_party/mpv/build/patches/libass-windows-header.patch'],
  ['shaderc', 2, 'desktop/third_party/mpv/build/patches/shaderc-glslang-install.patch'],
  ['ffmpeg', 3, 'desktop/third_party/mpv/build/patches/ffmpeg-llvm-lib-msvc.patch'],
  ['libplacebo', 4, 'desktop/third_party/mpv/build/patches/libplacebo-vulkan-surface-capabilities-fallback.patch'],
]);
const RUNTIME_ARTIFACT_ROLES = Object.freeze([
  'mpv-executable', 'spirv-cross-runtime', 'vulkan-loader-runtime',
]);

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function exactKeys(value, expected, path) {
  assert(value && typeof value === 'object' && !Array.isArray(value), `${path} 必须是对象`);
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  assert(
    actual.length === wanted.length && actual.every((key, index) => key === wanted[index]),
    `${path} 字段必须精确匹配固定契约`,
  );
}

function exactStringSet(value, expected, path) {
  assert(Array.isArray(value) && value.every((item) => typeof item === 'string'), `${path} 必须是字符串数组`);
  const actual = [...new Set(value)].sort();
  const wanted = [...expected].sort();
  assert(actual.length === value.length, `${path} 不允许重复项`);
  assert(
    actual.length === wanted.length && actual.every((item, index) => item === wanted[index]),
    `${path} 必须精确匹配固定集合`,
  );
}

function relativePath(value, path) {
  assert(typeof value === 'string' && value.length > 0, `${path} 必须是非空相对路径`);
  assert(
    !value.includes('\\') && !value.startsWith('/') && !value.includes(':'),
    `${path} 必须使用正斜杠相对路径`,
  );
  assert(
    value.split('/').every((segment) => segment && segment !== '.' && segment !== '..'),
    `${path} 包含无效路径段`,
  );
}

function httpsUrl(value, path) {
  assert(typeof value === 'string', `${path} 必须是 HTTPS`);
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error(`${path} 必须是有效 HTTPS URL`);
  }
  assert(
    parsed.protocol === 'https:' && parsed.hostname && !parsed.username && !parsed.password,
    `${path} 必须是无凭证 HTTPS URL`,
  );
  assert(!parsed.search && !parsed.hash, `${path} 不允许查询或片段`);
  assert(
    !/(?:^|[._\/-])(?:latest|main|master|head)(?:[._\/-]|$)/i.test(parsed.pathname),
    `${path} 不允许移动引用`,
  );
}

function cachePair(value, blockers, path, prefix) {
  assert(value.cache_path === null || typeof value.cache_path === 'string', `${path}.cache_path 无效`);
  assert(
    value.size_bytes === null
      || (Number.isSafeInteger(value.size_bytes) && value.size_bytes > 0 && value.size_bytes <= 8 * 1024 * 1024 * 1024),
    `${path}.size_bytes 无效`,
  );
  assert(value.sha256 === null || SHA256.test(value.sha256), `${path}.sha256 无效`);
  assert(
    (value.cache_path === null) === (value.sha256 === null)
      && (value.cache_path === null) === (value.size_bytes === null),
    `${path}.cache_path、size_bytes 与 sha256 必须同时存在或同时为空`,
  );
  if (value.cache_path === null) {
    blockers.push(`${path} 缺少离线缓存路径和 SHA-256`);
    return;
  }
  relativePath(value.cache_path, `${path}.cache_path`);
  assert(value.cache_path.startsWith(`${prefix}/`), `${path}.cache_path 必须位于 ${prefix}/`);
}

function requiredBuildArgument(feature, enabled) {
  if (feature === 'cplayer') return '-Dcplayer=true';
  if (feature === 'libmpv') return '-Dlibmpv=false';
  return `-D${feature}=${enabled ? 'enabled' : 'disabled'}`;
}

function validateArguments(lock, blockers) {
  const {
    meson_arguments: meson,
    spirv_cross_cmake_arguments: spirvCrossCmake,
    ffmpeg_arguments: ffmpeg,
  } = lock.build_recipe;
  assert(Array.isArray(meson) && meson.every((item) => typeof item === 'string'), 'build_recipe.meson_arguments 无效');
  assert(Array.isArray(ffmpeg) && ffmpeg.every((item) => typeof item === 'string'), 'build_recipe.ffmpeg_arguments 无效');
  assert(new Set(meson).size === meson.length, 'build_recipe.meson_arguments 不允许重复');
  assert(new Set(ffmpeg).size === ffmpeg.length, 'build_recipe.ffmpeg_arguments 不允许重复');
  assert(Array.isArray(spirvCrossCmake) && spirvCrossCmake.every((item) => typeof item === 'string'), 'build_recipe.spirv_cross_cmake_arguments 无效');
  assert(new Set(spirvCrossCmake).size === spirvCrossCmake.length, 'build_recipe.spirv_cross_cmake_arguments 不允许重复');
  const forbidden = /(?:https?:\/\/|\b(?:git|curl|wget)\b|invoke-webrequest|git-sync-deps|\b(?:master|main|latest)\b)/i;
  assert(![...meson, ...spirvCrossCmake, ...ffmpeg].some((argument) => forbidden.test(argument)), '构建参数禁止联网或移动引用');
  const requiredMeson = [
    '--wrap-mode=nodownload', '-Dauto_features=disabled', '-Dbuild-date=false',
    '-Ddefault_library=static',
    ...REQUIRED_FEATURES.map((feature) => requiredBuildArgument(feature, true)),
    ...DISABLED_FEATURES.map((feature) => requiredBuildArgument(feature, false)),
    ...DISABLED_BOOLEAN_OPTIONS.map((option) => `-D${option}=false`),
  ];
  const allowedMeson = new Set(requiredMeson);
  assert(meson.every((argument) => allowedMeson.has(argument)), 'build_recipe.meson_arguments 包含未知或覆盖参数');
  for (const argument of requiredMeson) {
    if (!meson.includes(argument)) blockers.push(`build_recipe.meson_arguments 缺少 ${argument}`);
  }
  const requiredSpirvCrossCmake = [
    '-DCMAKE_BUILD_TYPE=Release',
    '-DCMAKE_POLICY_DEFAULT_CMP0091=NEW',
    '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded',
    '-DBUILD_SHARED_LIBS=OFF',
    '-DSPIRV_CROSS_SHARED=ON',
    '-DSPIRV_CROSS_CLI=OFF',
    '-DSPIRV_CROSS_ENABLE_TESTS=OFF',
    '-DSPIRV_CROSS_ENABLE_MSL=OFF',
    '-DSPIRV_CROSS_ENABLE_CPP=OFF',
    '-DSPIRV_CROSS_ENABLE_REFLECT=OFF',
    '-DSPIRV_CROSS_ENABLE_UTIL=OFF',
  ];
  const allowedSpirvCrossCmake = new Set(requiredSpirvCrossCmake);
  assert(
    spirvCrossCmake.every((argument) => allowedSpirvCrossCmake.has(argument)),
    'build_recipe.spirv_cross_cmake_arguments 包含未知或覆盖参数',
  );
  for (const argument of requiredSpirvCrossCmake) {
    if (!spirvCrossCmake.includes(argument)) {
      blockers.push(`build_recipe.spirv_cross_cmake_arguments 缺少 ${argument}`);
    }
  }
  const requiredFfmpeg = [
    '--target-os=win32', '--arch=x86_64', '--toolchain=msvc', '--enable-static', '--enable-cross-compile',
    '--disable-shared', '--disable-autodetect', '--disable-debug', '--disable-doc',
    '--disable-network', '--disable-programs', '--enable-gpl', '--enable-d3d11va',
    ...['av1', 'h264', 'hevc', 'mpeg2', 'vc1', 'vp9', 'wmv3']
      .flatMap((codec) => [`--enable-hwaccel=${codec}_d3d11va`, `--enable-hwaccel=${codec}_d3d11va2`]),
  ];
  const allowedFfmpeg = new Set(requiredFfmpeg);
  assert(ffmpeg.every((argument) => allowedFfmpeg.has(argument)), 'build_recipe.ffmpeg_arguments 包含未知或覆盖参数');
  for (const argument of requiredFfmpeg) {
    if (!ffmpeg.includes(argument)) blockers.push(`build_recipe.ffmpeg_arguments 缺少 ${argument}`);
  }
  const ffmpegSource = lock.sources.find(({ name }) => name === 'ffmpeg');
  assert(
    ffmpegSource?.license_expression === 'GPL-2.0-or-later',
    'FFmpeg 启用 --enable-gpl 后许可证身份必须为 GPL-2.0-or-later',
  );
}

function validateSource(source, index, names, blockers) {
  const path = `sources[${index}]`;
  assert(source && typeof source === 'object' && !Array.isArray(source), `${path} 必须是对象`);
  assert(source.kind === 'git' || source.kind === 'archive', `${path}.kind 只允许 git/archive`);
  const identityKeys = source.kind === 'git'
    ? ['kind', 'name', 'repository', 'commit', 'cache_path', 'size_bytes', 'sha256', 'license_expression', 'usage', 'runtime_artifacts']
    : ['kind', 'name', 'url', 'cache_path', 'size_bytes', 'sha256', 'license_expression', 'usage', 'runtime_artifacts'];
  exactKeys(source, identityKeys, path);
  assert(typeof source.name === 'string' && /^[a-z0-9][a-z0-9+.-]*$/.test(source.name), `${path}.name 无效`);
  assert(!names.has(source.name), `sources 包含重复组件：${source.name}`);
  names.add(source.name);
  if (source.kind === 'git') {
    httpsUrl(source.repository, `${path}.repository`);
    assert(COMMIT.test(source.commit), `${path}.commit 必须是 40 位提交`);
  } else {
    httpsUrl(source.url, `${path}.url`);
  }
  cachePair(source, blockers, path, 'sources');
  assert(typeof source.license_expression === 'string' && source.license_expression.length > 0, `${path}.license_expression 无效`);
  if (source.license_expression === 'NOASSERTION') blockers.push(`${path}.license_expression 尚未复核`);
  assert(source.usage === 'build' || source.usage === 'runtime', `${path}.usage 只允许 build/runtime`);
  assert(Array.isArray(source.runtime_artifacts), `${path}.runtime_artifacts 必须是数组`);
  exactStringSet(source.runtime_artifacts, source.runtime_artifacts, `${path}.runtime_artifacts`);
  assert(
    source.runtime_artifacts.every((role) => RUNTIME_ARTIFACT_ROLES.includes(role)),
    `${path}.runtime_artifacts 包含未知运行时角色`,
  );
  assert(
    source.usage === 'runtime' ? source.runtime_artifacts.length > 0 : source.runtime_artifacts.length === 0,
    `${path}.usage 与 runtime_artifacts 不一致`,
  );
}

function validateToolchain(toolchain, blockers) {
  if (toolchain === null) {
    blockers.push('toolchain 尚未固定');
    return [];
  }
  exactKeys(toolchain, ['builder_image', 'dockerfile_path', 'dockerfile_sha256', 'tools'], 'toolchain');
  assert(/^[^\s]+@sha256:[a-f0-9]{64}$/.test(toolchain.builder_image), 'toolchain.builder_image 必须固定镜像摘要');
  relativePath(toolchain.dockerfile_path, 'toolchain.dockerfile_path');
  assert(toolchain.dockerfile_path.startsWith('desktop/third_party/mpv/build/'), 'Dockerfile 必须位于固定构建目录');
  assert(SHA256.test(toolchain.dockerfile_sha256), 'toolchain.dockerfile_sha256 无效');
  exactKeys(toolchain.tools, REQUIRED_TOOLS, 'toolchain.tools');
  const cachePaths = [];
  for (const name of REQUIRED_TOOLS) {
    const tool = toolchain.tools[name];
    const path = `toolchain.tools.${name}`;
    exactKeys(tool, ['version', 'source', 'license_expression', 'provision', 'cache_path', 'size_bytes', 'sha256'], path);
    assert(typeof tool.version === 'string' && tool.version.trim().length > 0, `${path}.version 为空`);
    httpsUrl(tool.source, `${path}.source`);
    assert(typeof tool.license_expression === 'string' && tool.license_expression.length > 0 && tool.license_expression !== 'NOASSERTION', `${path}.license_expression 尚未复核`);
    assert(tool.provision === 'builder_image' || tool.provision === 'cache', `${path}.provision 无效`);
    if (tool.provision === 'builder_image') {
      assert(
        tool.cache_path === null && tool.size_bytes === null && tool.sha256 === null,
        `${path} 由 builder image 提供时不得声明独立缓存`,
      );
    } else {
      cachePair(tool, blockers, path, 'toolchain');
      if (tool.cache_path !== null) cachePaths.push(tool.cache_path);
    }
  }
  return cachePaths;
}

export function verifyReproducibleBuildLock(lock) {
  exactKeys(lock, [
    'schema_version', 'target', 'fetch_policy', 'sources', 'patches', 'cache_inventory',
    'recipe_inventory', 'toolchain', 'build_recipe', 'features', 'output_policy',
  ], '$');
  assert(lock.schema_version === 1, 'schema_version 必须为 1');
  assert(lock.target === 'x86_64-pc-windows-msvc', 'target 必须固定为 x86_64-pc-windows-msvc');
  exactKeys(lock.fetch_policy, ['network', 'build_network', 'source_mode'], 'fetch_policy');
  assert(lock.fetch_policy.network === 'https_only', 'fetch_policy.network 必须为 https_only');
  assert(lock.fetch_policy.build_network === 'none', 'fetch_policy.build_network 必须为 none');
  assert(lock.fetch_policy.source_mode === 'locked_cache_only', 'fetch_policy.source_mode 必须为 locked_cache_only');

  const blockers = [];
  assert(Array.isArray(lock.sources) && lock.sources.length >= REQUIRED_SOURCES.length, 'sources 不完整');
  const names = new Set();
  const cachePaths = [];
  lock.sources.forEach((source, index) => {
    validateSource(source, index, names, blockers);
    if (source.cache_path !== null) cachePaths.push(source.cache_path);
  });
  for (const name of REQUIRED_SOURCES) assert(names.has(name), `sources 缺少 ${name}`);
  assert(Array.isArray(lock.patches), 'patches 必须是数组');
  assert(lock.patches.length === REQUIRED_PATCHES.length, 'patches 必须精确声明五个生产补丁');
  const patchOrders = new Set();
  lock.patches.forEach((patch, index) => {
    const path = `patches[${index}]`;
    exactKeys(patch, ['component', 'order', 'path', 'size_bytes', 'sha256'], path);
    assert(names.has(patch.component), `${path}.component 不在 sources`);
    assert(Number.isInteger(patch.order) && patch.order >= 0, `${path}.order 无效`);
    assert(!patchOrders.has(patch.order), 'patches.order 不允许重复');
    patchOrders.add(patch.order);
    relativePath(patch.path, `${path}.path`);
    assert(patch.path.startsWith('desktop/third_party/mpv/build/patches/'), `${path}.path 必须位于固定配方补丁目录`);
    assert(SHA256.test(patch.sha256), `${path}.sha256 无效`);
    assert(Number.isSafeInteger(patch.size_bytes) && patch.size_bytes > 0, `${path}.size_bytes 无效`);
  });
  const actualPatches = lock.patches.map((patch) => [patch.component, patch.order, patch.path]);
  assert(
    actualPatches.every((patch, index) => patch.every((value, field) => value === REQUIRED_PATCHES[index][field])),
    'patches 的组件、顺序和路径必须精确匹配生产配方',
  );
  cachePaths.push(...validateToolchain(lock.toolchain, blockers));

  exactKeys(lock.build_recipe, [
    'script_path', 'script_sha256', 'meson_arguments', 'spirv_cross_cmake_arguments',
    'ffmpeg_arguments', 'environment',
  ], 'build_recipe');
  assert(lock.build_recipe.script_path === null || typeof lock.build_recipe.script_path === 'string', 'build_recipe.script_path 无效');
  assert(lock.build_recipe.script_sha256 === null || SHA256.test(lock.build_recipe.script_sha256), 'build_recipe.script_sha256 无效');
  assert(
    (lock.build_recipe.script_path === null) === (lock.build_recipe.script_sha256 === null),
    'build_recipe 脚本路径和哈希必须同时存在或同时为空',
  );
  if (lock.build_recipe.script_path === null) {
    blockers.push('build_recipe 尚未固定断网构建脚本');
  } else {
    relativePath(lock.build_recipe.script_path, 'build_recipe.script_path');
    assert(
      lock.build_recipe.script_path.startsWith('desktop/third_party/mpv/build/'),
      '构建脚本必须位于固定构建目录',
    );
  }
  exactKeys(lock.build_recipe.environment, [
    'source_date_epoch', 'locale', 'timezone', 'path_prefix_map',
  ], 'build_recipe.environment');
  assert(lock.build_recipe.environment.locale === 'C.UTF-8', '构建 locale 必须为 C.UTF-8');
  assert(lock.build_recipe.environment.timezone === 'UTC', '构建 timezone 必须为 UTC');
  if (!Number.isSafeInteger(lock.build_recipe.environment.source_date_epoch)
      || lock.build_recipe.environment.source_date_epoch < 946684800
      || lock.build_recipe.environment.source_date_epoch > 4102444800) {
    blockers.push('SOURCE_DATE_EPOCH 尚未固定');
  }
  if (lock.build_recipe.environment.path_prefix_map !== '/work=/usr/src/autolive-mpv') {
    blockers.push('构建路径前缀映射尚未固定');
  }
  validateArguments(lock, blockers);

  assert(Array.isArray(lock.recipe_inventory), 'recipe_inventory 必须是数组');
  const recipePaths = new Set();
  for (const [index, recipe] of lock.recipe_inventory.entries()) {
    const path = `recipe_inventory[${index}]`;
    exactKeys(recipe, ['path', 'size_bytes', 'sha256'], path);
    relativePath(recipe.path, `${path}.path`);
    assert(recipe.path.startsWith('desktop/third_party/mpv/build/'), `${path}.path 必须位于固定构建目录`);
    assert(!recipePaths.has(recipe.path), 'recipe_inventory 不允许重复路径');
    recipePaths.add(recipe.path);
    assert(Number.isSafeInteger(recipe.size_bytes) && recipe.size_bytes > 0 && recipe.size_bytes <= 4 * 1024 * 1024, `${path}.size_bytes 无效`);
    assert(SHA256.test(recipe.sha256), `${path}.sha256 无效`);
  }
  const recipes = new Map(lock.recipe_inventory.map((recipe) => [recipe.path, recipe]));
  for (const patch of lock.patches) {
    const recipe = recipes.get(patch.path);
    assert(recipe?.size_bytes === patch.size_bytes && recipe?.sha256 === patch.sha256, `补丁未与 recipe_inventory 双向锁定：${patch.path}`);
  }
  if (lock.toolchain !== null && lock.build_recipe.script_path !== null) {
    assert(lock.toolchain.dockerfile_path !== lock.build_recipe.script_path, 'Dockerfile 与构建脚本不能是同一文件');
    const dockerfile = recipes.get(lock.toolchain.dockerfile_path);
    const script = recipes.get(lock.build_recipe.script_path);
    assert(dockerfile?.sha256 === lock.toolchain.dockerfile_sha256, 'recipe_inventory 缺少匹配的 Dockerfile');
    assert(script?.sha256 === lock.build_recipe.script_sha256, 'recipe_inventory 缺少匹配的构建脚本');
  }

  exactKeys(lock.features, [
    'auto_features', 'build_date', 'required', 'disabled', 'disabled_boolean_options',
  ], 'features');
  assert(lock.features.auto_features === 'disabled', 'features.auto_features 必须为 disabled');
  assert(lock.features.build_date === false, 'features.build_date 必须为 false');
  exactStringSet(lock.features.required, REQUIRED_FEATURES, 'features.required');
  exactStringSet(lock.features.disabled, DISABLED_FEATURES, 'features.disabled');
  exactStringSet(
    lock.features.disabled_boolean_options,
    DISABLED_BOOLEAN_OPTIONS,
    'features.disabled_boolean_options',
  );

  exactKeys(lock.output_policy, ['required_roles', 'dynamic_dependencies_allowlist', 'bundled_dynamic_dependencies'], 'output_policy');
  exactStringSet(lock.output_policy.required_roles, REQUIRED_OUTPUT_ROLES, 'output_policy.required_roles');
  if (lock.output_policy.dynamic_dependencies_allowlist === null) {
    blockers.push('PE 动态依赖允许列表尚未固定');
  } else {
    assert(Array.isArray(lock.output_policy.dynamic_dependencies_allowlist), '动态依赖允许列表必须是数组');
    const dlls = lock.output_policy.dynamic_dependencies_allowlist;
    assert(dlls.every((name) => /^[A-Z0-9._-]+\.DLL$/.test(name)), '动态依赖必须是大写 DLL 文件名');
    assert(new Set(dlls).size === dlls.length, '动态依赖允许列表不允许重复');
  }
  exactKeys(
    lock.output_policy.bundled_dynamic_dependencies,
    ['SPIRV-CROSS-C-SHARED.DLL', 'VULKAN-1.DLL'],
    'output_policy.bundled_dynamic_dependencies',
  );
  assert(
    lock.output_policy.bundled_dynamic_dependencies['SPIRV-CROSS-C-SHARED.DLL'] === 'spirv-cross-runtime'
      && lock.output_policy.bundled_dynamic_dependencies['VULKAN-1.DLL'] === 'vulkan-loader-runtime',
    '随包动态依赖与产物角色映射不正确',
  );
  for (const dll of Object.keys(lock.output_policy.bundled_dynamic_dependencies)) {
    assert(lock.output_policy.dynamic_dependencies_allowlist?.includes(dll), `随包 DLL 未进入动态依赖允许列表：${dll}`);
  }

  assert(Array.isArray(lock.cache_inventory) && lock.cache_inventory.every((item) => typeof item === 'string'), 'cache_inventory 必须是字符串数组');
  lock.cache_inventory.forEach((item, index) => relativePath(item, `cache_inventory[${index}]`));
  const actualInventory = [...new Set(lock.cache_inventory)].sort();
  assert(actualInventory.length === lock.cache_inventory.length, 'cache_inventory 不允许重复');
  const expectedInventory = [...new Set(cachePaths)].sort();
  assert(expectedInventory.length === cachePaths.length, '源码和工具链不允许复用缓存路径');
  if (actualInventory.length !== expectedInventory.length
      || actualInventory.some((item, index) => item !== expectedInventory[index])) {
    blockers.push('cache_inventory 与源码、补丁、工具链缓存不完全一致');
  }

  return {
    schemaVersion: 1,
    gate: 'mpv-reproducible-build-lock',
    checkStatus: 'passed',
    status: blockers.length === 0 ? 'metadata_complete' : 'blocked',
    metadataComplete: blockers.length === 0,
    target: lock.target,
    sourceCount: lock.sources.length,
    blockers,
  };
}

async function main() {
  const args = process.argv.slice(2);
  assert(
    args.length <= 1 && (args.length === 0 || args[0] === '--require-metadata-complete'),
    '只允许可选参数 --require-metadata-complete',
  );
  const report = verifyReproducibleBuildLock(JSON.parse(await readFile(lockPath, 'utf8')));
  console.log(JSON.stringify(report, null, 2));
  if (args[0] === '--require-metadata-complete' && !report.metadataComplete) process.exitCode = 2;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
