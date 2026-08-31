const TOOL_NAMES = Object.freeze([
  'clang', 'cmake', 'lld-link', 'llvm-rc', 'meson', 'nasm', 'ninja', 'pkg-config',
  'python', 'ucrt', 'windows-sdk', 'msvc-toolset',
]);

export function completeBuildLockFixture(input) {
  const lock = structuredClone(input);
  lock.sources = lock.sources.map((source, index) => ({
    ...source,
    cache_path: `sources/${source.name}.tar.zst`,
    size_bytes: index + 1,
    sha256: (index + 1).toString(16).padStart(2, '0').repeat(32),
    license_expression: source.license_expression === 'NOASSERTION'
      ? 'GPL-3.0-or-later'
      : source.license_expression,
    usage: source.usage ?? 'runtime',
    runtime_artifacts: source.runtime_artifacts ?? ['mpv-executable'],
  }));
  if (!lock.sources.some((source) => source.name === 'shaderc')) {
    lock.sources.push({
      kind: 'archive',
      name: 'shaderc',
      url: 'https://example.invalid/shaderc.tar.zst',
      cache_path: 'sources/shaderc.tar.zst',
      size_bytes: 4,
      sha256: '4'.repeat(64),
      license_expression: 'Apache-2.0',
    });
  }
  lock.toolchain = {
    builder_image: `ghcr.io/example/mpv-builder@sha256:${'b'.repeat(64)}`,
    dockerfile_path: 'desktop/third_party/mpv/build/Dockerfile',
    dockerfile_sha256: 'c'.repeat(64),
    tools: Object.fromEntries(TOOL_NAMES.map((name, index) => {
      const inBuilderImage = ['clang', 'cmake', 'lld-link', 'ninja', 'python'].includes(name);
      return [name, {
        version: `1.${index}.0`,
        source: `https://example.invalid/toolchain/${name}`,
        license_expression: 'Apache-2.0',
        provision: inBuilderImage ? 'builder_image' : 'cache',
        cache_path: inBuilderImage ? null : `toolchain/${name}.tar.zst`,
        size_bytes: inBuilderImage ? null : index + 1,
        sha256: inBuilderImage ? null : (index + 1).toString(16).repeat(64),
      }];
    })),
  };
  lock.build_recipe = {
    script_path: 'desktop/third_party/mpv/build/build.ps1',
    script_sha256: 'd'.repeat(64),
    meson_arguments: [
      '--wrap-mode=nodownload',
      '-Dauto_features=disabled',
      '-Dbuild-date=false',
      '-Ddefault_library=static',
      '-Dfuzzers=false',
      '-Dtests=false',
      '-Dcplayer=true',
      '-Dd3d11=enabled',
      '-Dd3d-hwaccel=enabled',
      '-Dlcms2=enabled',
      '-Dshaderc=enabled',
      '-Dspirv-cross=enabled',
      '-Dvulkan=enabled',
      '-Dwin32-threads=enabled',
      '-Dcdda=disabled',
      '-Dcplugins=disabled',
      '-Ddvbin=disabled',
      '-Ddvdnav=disabled',
      '-Dgl=disabled',
      '-Djavascript=disabled',
      '-Dlibarchive=disabled',
      '-Dlibavdevice=disabled',
      '-Dlibbluray=disabled',
      '-Dlibcurl=disabled',
      '-Dlibmpv=false',
      '-Dlua=disabled',
      '-Dmanpage-build=disabled',
      '-Dopenal=disabled',
      '-Dsdl2-audio=disabled',
      '-Dsdl2-gamepad=disabled',
      '-Dsdl2-video=disabled',
      '-Dsixel=disabled',
      '-Dsubrandr=disabled',
      '-Dvapoursynth=disabled',
      '-Dwasapi=disabled',
    ],
    spirv_cross_cmake_arguments: [
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
    ],
    ffmpeg_arguments: [
      '--target-os=win32', '--arch=x86_64', '--toolchain=msvc', '--enable-static', '--enable-cross-compile',
      '--disable-shared', '--disable-autodetect', '--disable-debug', '--disable-doc',
      '--disable-network', '--disable-programs', '--enable-gpl', '--enable-d3d11va',
      ...['av1', 'h264', 'hevc', 'mpeg2', 'vc1', 'vp9', 'wmv3']
        .flatMap((codec) => [`--enable-hwaccel=${codec}_d3d11va`, `--enable-hwaccel=${codec}_d3d11va2`]),
    ],
    environment: {
      source_date_epoch: 1787875200,
      locale: 'C.UTF-8',
      timezone: 'UTC',
      path_prefix_map: '/work=/usr/src/autolive-mpv',
    },
  };
  lock.recipe_inventory = [
    {
      path: lock.toolchain.dockerfile_path,
      size_bytes: 1,
      sha256: lock.toolchain.dockerfile_sha256,
    },
    {
      path: lock.build_recipe.script_path,
      size_bytes: 1,
      sha256: lock.build_recipe.script_sha256,
    },
    ...lock.patches.map((patch) => ({
      path: patch.path,
      size_bytes: patch.size_bytes,
      sha256: patch.sha256,
    })),
  ];
  lock.output_policy.dynamic_dependencies_allowlist = [
    'KERNEL32.DLL', 'SPIRV-CROSS-C-SHARED.DLL', 'USER32.DLL', 'VULKAN-1.DLL',
  ];
  lock.output_policy.bundled_dynamic_dependencies = {
    'SPIRV-CROSS-C-SHARED.DLL': 'spirv-cross-runtime',
    'VULKAN-1.DLL': 'vulkan-loader-runtime',
  };
  lock.cache_inventory = [
    ...lock.sources.map((source) => source.cache_path),
    ...Object.values(lock.toolchain.tools).flatMap((tool) => tool.cache_path ? [tool.cache_path] : []),
  ].sort();
  return lock;
}
