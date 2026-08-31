import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const buildRoot = new URL('../third_party/mpv/build/', import.meta.url);

async function recipe(name) {
  return readFile(new URL(name, buildRoot), 'utf8');
}

test('Phase 7A Docker 配方固定官方 LLVM 镜像且不包含联网安装入口', async () => {
  const source = await recipe('Dockerfile');
  assert.match(source, /^FROM ghcr\.io\/llvm\/ci-ubuntu-24\.04@sha256:224c58f5d5f3f1d4b8f36dd3873b00a5d60c28065693165d875a9454ed914233$/m);
  assert.doesNotMatch(source, /\b(?:ADD|RUN)\b|apt(?:-get)?|pip\s+install|curl|wget|git\s+clone/i);
  assert.match(source, /ENTRYPOINT \["\/bin\/bash", "\/recipe\/build\.sh"\]/);
});

test('Phase 7A 构建脚本只接受断网、固定输入并在闭包未准入时失败关闭', async () => {
  const source = await recipe('build.sh');
  assert.match(source, /set -Eeuo pipefail/);
  assert.match(source, /\/build-inputs/);
  assert.match(source, /\/out/);
  assert.match(source, /x86_64-pc-windows-msvc/);
  assert.match(source, /\/proc\/net\/route/);
  assert.match(source, /prepare-inputs\.py/);
  assert.match(source, /build-dependencies\.sh/);
  assert.match(source, /generate-evidence\.py/);
  assert.doesNotMatch(source, /dependency-closure-not-admitted/);
  assert.doesNotMatch(source, /https?:\/\/|git\s+(?:clone|fetch|pull|submodule)|curl|wget|Invoke-WebRequest/i);
  assert.doesNotMatch(source, /mingw|x86_64-w64-windows-gnu/i);
});

test('Phase 7A FFmpeg 配方显式启用 CPU4 eq 与常用 D3D11VA 硬解', async () => {
  const source = await recipe('build-dependencies.sh');
  for (const argument of [
    '--enable-gpl',
    '--enable-d3d11va',
    '--enable-hwaccel=av1_d3d11va',
    '--enable-hwaccel=av1_d3d11va2',
    '--enable-hwaccel=h264_d3d11va',
    '--enable-hwaccel=h264_d3d11va2',
    '--enable-hwaccel=hevc_d3d11va',
    '--enable-hwaccel=hevc_d3d11va2',
    '--enable-hwaccel=mpeg2_d3d11va',
    '--enable-hwaccel=mpeg2_d3d11va2',
    '--enable-hwaccel=vc1_d3d11va',
    '--enable-hwaccel=vc1_d3d11va2',
    '--enable-hwaccel=vp9_d3d11va',
    '--enable-hwaccel=vp9_d3d11va2',
    '--enable-hwaccel=wmv3_d3d11va',
    '--enable-hwaccel=wmv3_d3d11va2',
  ]) {
    assert.match(source, new RegExp(`^\\s*${argument.replaceAll(/[.*+?^${}()|[\\]\\\\]/g, '\\$&')} \\\\?$`, 'm'));
  }
});

test('Phase 7A 构建固定低并发且冷构建容器限制为 4 GiB', async () => {
  const [dependencies, runner] = await Promise.all([
    recipe('build-dependencies.sh'),
    recipe('run-locked-cold-build.ps1'),
  ]);
  assert.match(dependencies, /^readonly JOBS=2$/m);
  assert.doesNotMatch(dependencies, /\bnproc\b/);
  const ninjaCommands = dependencies.match(/^ninja -C .*$/gm) ?? [];
  assert.ok(ninjaCommands.length > 0);
  assert.ok(ninjaCommands.every((command) => command.includes('-j "$JOBS"')));
  assert.match(runner, /--memory '4g'/);
  assert.match(runner, /--memory-swap '4g'/);
  assert.match(runner, /memoryBytes = \[int64\]\$before\.HostConfig\.Memory/);
  assert.match(runner, /memorySwapBytes = \[int64\]\$before\.HostConfig\.MemorySwap/);
});

test('Phase 7A 冷构建 runner 每次创建并保留唯一 nocopy work volume', async () => {
  const source = await recipe('run-locked-cold-build.ps1');
  assert.match(source, /\[Guid\]::NewGuid\(\)\.ToString\('N'\)/);
  assert.match(source, /docker volume inspect \$workVolumeName/);
  assert.match(source, /docker volume create/);
  assert.match(source, /--user '0:0'/);
  assert.match(source, /type=volume,src=\$workVolumeName,dst=\/work,volume-nocopy/);
  assert.match(source, /type=bind,src=\$inputRoot,dst=\/build-inputs,readonly/);
  assert.match(source, /type=bind,src=\$OutputRoot,dst=\/out/);
  assert.match(source, /existedBeforeCreate = \$false/);
  assert.match(source, /retained = \$true/);
  assert.doesNotMatch(source, /OutputRoot-work|\$workRoot|docker volume (?:rm|remove)/i);
});

test('Phase 7A 输入清单列出 MSVC sysroot、缺失 LLVM 工具和 GPU 主链直接依赖', async () => {
  const entries = (await recipe('required-inputs.txt'))
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith('#'));
  const expected = [
    'sources/mpv-7b8915bc1d04c7e1b61184e00c7fbfaab1911e75.tar.gz',
    'sources/ffmpeg-1d7b14f61d66fdf18f15204c613df9d65396c319.tar.gz',
    'sources/libplacebo-22ee762e8e0890fc54068beb670310f0edce7263.tar.gz',
    'sources/shaderc-7060a6615a1c6e2515e696651eea685524ecadb5.tar.gz',
    'sources/glslang-2ee090f606ace31e07f584b1c1b9ddf4909ce202.tar.gz',
    'sources/spirv-tools-f589ef005c49f6f19c8e78eb5269104ba293beb4.tar.gz',
    'sources/spirv-headers-942fe4b988359a0750b79f0ae7ed735994d3147d.tar.gz',
    'sources/spirv-cross-9c3c8e2cefdd8194b193bb8ed2fdff4d5527e382.tar.gz',
    'sources/vulkan-headers-74d8a6cb930c68ef617b202c3ff3c59d919e086b.tar.gz',
    'sources/vulkan-loader-363f465abadab0a8dcfc5c85d2c691e9b0b788d6.tar.gz',
    'sources/lcms2-21c582a594fe5279f90c0b93437c398f93bf62b0.tar.gz',
    'sources/libass-4a05d8127f525943ebf45fdc6497c9e665947f0d.tar.gz',
    'sources/freetype-526ec5c47b9ebccc4754c85ac0c0cdf7c85a5e9b.tar.gz',
    'sources/fribidi-68162babff4f39c4e2dc164a5e825af93bda9983.tar.gz',
    'sources/harfbuzz-36cb489cb02ce4b92099669ba9f9bea348eff93f.tar.gz',
    'sources/fast-float-97b54ca9e75f5303507699d27c6b4f4efe4641a1.tar.gz',
    'sources/jinja-15206881c006c79667fe5154fe80c01c65410679.tar.gz',
    'sources/markupsafe-297fc8e356e6836a62087949245d09a28e9f1b13.tar.gz',
    'sources/zstd-1.5.7.tar.gz',
    'toolchain/meson-1.9.0.tar.gz',
    'toolchain/nasm-2.16.03.tar.xz',
    'toolchain/pkgconf-2.5.1.tar.xz',
    'toolchain/llvm-project-22.1.4.src.tar.xz',
    'toolchain/windows-sdk-10.0.26100.0.tar.gz',
    'toolchain/ucrt-10.0.26100.0.tar.gz',
    'toolchain/msvc-toolset-14.44.35207.tar.gz',
  ];
  assert.deepEqual(entries, expected);
});

test('Phase 7A 配方包含安全解包、真实依赖构建和证据生成三个独立边界', async () => {
  const [prepare, dependencies, evidence] = await Promise.all([
    recipe('prepare-inputs.py'),
    recipe('build-dependencies.sh'),
    recipe('generate-evidence.py'),
  ]);
  assert.match(prepare, /tarfile/);
  assert.match(prepare, /filter=['"]data['"]/);
  assert.match(prepare, /lock\['patches'\]/);
  assert.match(prepare, /sorted\(lock\['patches'\], key=lambda item: item\['order'\]\)/);
  assert.match(prepare, /verify_file\(recipe_patch, patch\['size_bytes'\], patch\['sha256'\]/);
  assert.match(dependencies, /llvm-rc/);
  assert.doesNotMatch(dependencies, /link\.exe/);
  assert.match(dependencies, /ffmpeg/);
  assert.match(dependencies, /libplacebo/);
  assert.match(dependencies, /-DGLSLANG_ENABLE_INSTALL=OFF/);
  assert.match(dependencies, /-DSPIRV_CROSS_SHARED=ON/);
  assert.doesNotMatch(dependencies, /-DSPIRV_CROSS_SHARED=OFF/);
  assert.match(dependencies, /-DCMAKE_POLICY_DEFAULT_CMP0091=NEW/);
  assert.match(dependencies, /spirv-cross-c-shared\.dll/);
  assert.match(dependencies, /vulkan-1\.dll/);
  assert.match(dependencies, /--enable-cross-compile/);
  assert.match(dependencies, /-Dgl=disabled/);
  assert.match(dependencies, /ninja -C "\$MESON_BUILD_ROOT\/build-mpv"[^\n]*mpv\.exe/);
  assert.doesNotMatch(dependencies, /MESON\[@\].*compile[^\n]*mpv\.exe/);
  assert.match(dependencies, /MESON\[@\].*setup/);
  assert.match(evidence, /CycloneDX/);
  assert.match(evidence, /SPDX-2\.3/);
  assert.match(evidence, /build_component_rows\(lock\)/);
  assert.doesNotMatch(evidence, /COMPONENTS\s*=/);
  assert.match(evidence, /pe-imports/);
  assert.match(evidence, /spirv-cross-runtime/);
  assert.match(evidence, /vulkan-loader-runtime/);
  assert.match(evidence, /INTROSPECTION_KINDS/);
  assert.doesNotMatch(`${prepare}\n${dependencies}\n${evidence}`, /git-sync-deps|git\s+clone|https?:\/\//i);
});

test('Phase 7A FFmpeg 补丁按 MSVC 语义调用 llvm-lib 并清理孤立 heredoc', async () => {
  const patch = await recipe('patches/ffmpeg-llvm-lib-msvc.patch');
  assert.match(patch, /warning: no input files, not writing output file/);
  assert.match(patch, /arflags="-nologo"/);
  assert.match(patch, /ar_o='-out:\$@'/);
  assert.match(patch, /^-#ifdef WINAPI_FAMILY$/m);
  assert.match(patch, /^-EOF$/m);
  assert.doesNotMatch(patch, /^\+.*(?:mingw|x86_64-w64-windows-gnu)/mi);
});

test('Phase 7A shaderc 补丁在配置期关闭 glslang 安装导出', async () => {
  const patch = await recipe('patches/shaderc-glslang-install.patch');
  assert.match(patch, /if\(SKIP_GLSLANG_INSTALL\)/);
  assert.match(patch, /set\(GLSLANG_ENABLE_INSTALL OFF\)/);
  assert.doesNotMatch(patch, /^\+.*(?:SPIRV-Tools-opt|shaderc_combined)/m);
});

test('Phase 7A FriBidi 补丁只修正 native generator 与 Windows 标准头探测', async () => {
  const patch = await recipe('patches/fribidi-native-compiler.patch');
  assert.match(patch, /native_cc = meson\.get_compiler\('c', native: true\)/);
  assert.match(patch, /host_machine\.system\(\) == 'windows' or cc\.has_header\(h\)/);
  assert.doesNotMatch(patch, /^\+.*(?:HAVE_STRINGS_H|HAVE_SYS_TIMES_H)/m);
});

test('Phase 7A libass 补丁只纠正 Windows 目标的 windows.h 探测', async () => {
  const patch = await recipe('patches/libass-windows-header.patch');
  assert.match(patch, /host_machine\.system\(\) == 'windows' or cc\.has_header\('windows\.h'/);
  assert.match(patch, /declare_dependency\(link_args: \['gdi32\.lib'\]\)/);
  assert.doesNotMatch(patch, /^\+.*CONFIG_DIRECTWRITE/m);
});

test('Phase 7A libplacebo 补丁只对 AMD 已观测的 VK_ERROR_UNKNOWN 使用基础 surface 查询回退', async () => {
  const patch = await recipe('patches/libplacebo-vulkan-surface-capabilities-fallback.patch');
  assert.match(patch, /VK_ERROR_UNKNOWN/);
  assert.match(patch, /GetPhysicalDeviceSurfaceCapabilitiesKHR/);
  assert.match(patch, /GetPhysicalDeviceSurfaceCapabilities2KHR/);
  assert.doesNotMatch(patch, /VK_ERROR_DEVICE_LOST|VK_ERROR_OUT_OF_(?:HOST|DEVICE)_MEMORY/);
});

test('Meson 与 CMake machine files 固定真实 Windows MSVC 目标', async () => {
  const [cross, native, cmake] = await Promise.all([
    recipe('x86_64-pc-windows-msvc.ini'),
    recipe('linux-native.ini'),
    recipe('x86_64-pc-windows-msvc.cmake'),
  ]);
  assert.match(cross, /c\s*=\s*'\/opt\/llvm\/bin\/clang-cl'/);
  assert.match(cross, /c_ld\s*=\s*'lld-link'/);
  assert.match(cross, /ar\s*=\s*'\/work\/native\/bin\/llvm-lib'/);
  assert.match(cross, /windres\s*=\s*'\/work\/native\/bin\/llvm-rc'/);
  assert.match(cross, /system\s*=\s*'windows'/);
  assert.match(cross, /cpu_family\s*=\s*'x86_64'/);
  assert.match(cross, /x86_64-pc-windows-msvc/);
  assert.match(cross, /c_args\s*=.*'-fuse-ld=lld-link'/);
  assert.match(cross, /cpp_args\s*=.*'-fuse-ld=lld-link'/);
  assert.match(cross, /c_link_args\s*=.*'\/subsystem:console'/);
  assert.match(cross, /cpp_link_args\s*=.*'\/subsystem:console'/);
  assert.doesNotMatch(cross, /^sys_root\s*=/m);
  assert.match(cross, /^pkg_config_libdir\s*=\s*'\/work\/prefix\/lib\/pkgconfig'$/m);
  assert.match(native, /python\s*=\s*'\/usr\/bin\/python3'/);
  assert.match(cmake, /set\(CMAKE_SYSTEM_NAME Windows\)/);
  assert.match(cmake, /set\(CMAKE_C_COMPILER_TARGET x86_64-pc-windows-msvc\)/);
  assert.match(cmake, /set\(CMAKE_LINKER lld-link\)/);
  assert.match(cmake, /set\(CMAKE_AR \/work\/native\/bin\/llvm-lib\)/);
  assert.match(cmake, /set\(CMAKE_RC_COMPILER \/work\/native\/bin\/llvm-rc\)/);
  assert.match(cmake, /set\(CMAKE_MT \/work\/native\/bin\/llvm-mt\)/);
  assert.match(cmake, /set\(CMAKE_POLICY_DEFAULT_CMP0091 NEW\)/);
  assert.match(cmake, /set\(CMAKE_MSVC_RUNTIME_LIBRARY MultiThreaded\)/);
  assert.doesNotMatch(`${cross}\n${native}\n${cmake}`, /mingw|x86_64-w64-windows-gnu/i);
});

test('Phase 7A 将 Meson 探测目录放在不会被 clang-cl 误判为斜杠选项的路径', async () => {
  const [dependencies, evidence] = await Promise.all([
    recipe('build-dependencies.sh'),
    recipe('generate-evidence.py'),
  ]);
  assert.match(dependencies, /MESON_BUILD_ROOT='\/tmp\/autolive-meson-build'/);
  assert.match(dependencies, /MESON_BUILD_ROOT.*build-/s);
  assert.doesNotMatch(dependencies, /\/work\/build-(?:freetype|fribidi|harfbuzz|libass|lcms2|libplacebo|mpv)/);
  assert.match(evidence, /DEFAULT_MESON_ROOT = Path\('\/tmp\/autolive-meson-build'\)/);
});
