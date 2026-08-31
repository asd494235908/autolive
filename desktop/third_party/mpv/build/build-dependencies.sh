#!/usr/bin/env bash
set -Eeuo pipefail

readonly JOBS=2
readonly SOURCES='/work/sources'
readonly NATIVE='/work/native'
readonly PREFIX='/work/prefix'
readonly CROSS='/recipe/x86_64-pc-windows-msvc.ini'
readonly NATIVE_FILE='/recipe/linux-native.ini'
readonly CMAKE_TOOLCHAIN='/recipe/x86_64-pc-windows-msvc.cmake'
readonly MESON_SOURCE='/work/toolchain/meson'
readonly MESON_BUILD_ROOT='/tmp/autolive-meson-build'
readonly MESON=(/usr/bin/python3 "$MESON_SOURCE/meson.py")

mkdir -p "$NATIVE/bin" "$PREFIX" "$MESON_BUILD_ROOT"

cmake -S /work/toolchain/llvm-project/llvm -B /work/build-native-llvm -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$NATIVE" \
  -DLLVM_TARGETS_TO_BUILD=X86 \
  -DLLVM_BUILD_TOOLS=ON \
  -DLLVM_ENABLE_TERMINFO=OFF \
  -DLLVM_ENABLE_ZLIB=OFF \
  -DLLVM_ENABLE_ZSTD=OFF \
  -DLLVM_INCLUDE_BENCHMARKS=OFF \
  -DLLVM_INCLUDE_DOCS=OFF \
  -DLLVM_INCLUDE_EXAMPLES=OFF \
  -DLLVM_INCLUDE_TESTS=OFF
ninja -C /work/build-native-llvm -j "$JOBS" \
  llvm-ar llvm-lib llvm-mt llvm-nm llvm-ranlib llvm-rc llvm-readobj llvm-strip
for tool in llvm-ar llvm-lib llvm-mt llvm-nm llvm-ranlib llvm-rc llvm-readobj llvm-strip; do
  install -m 0755 "/work/build-native-llvm/bin/$tool" "$NATIVE/bin/$tool"
done

pushd /work/toolchain/pkgconf >/dev/null
./configure --prefix="$NATIVE" --disable-shared --enable-static
make -j"$JOBS"
make install
ln -sfn pkgconf "$NATIVE/bin/pkg-config"
popd >/dev/null

pushd /work/toolchain/nasm >/dev/null
./configure --prefix="$NATIVE"
make -j"$JOBS"
make install
popd >/dev/null

make -C "$SOURCES/zstd" -j"$JOBS" zstd-release
install -m 0755 "$SOURCES/zstd/programs/zstd" "$NATIVE/bin/zstd"

export PATH="$NATIVE/bin:/opt/llvm/bin:/usr/bin:/bin"
export PKG_CONFIG="$NATIVE/bin/pkg-config"
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"
export INCLUDE='/work/sysroot/msvc/include;/work/sysroot/ucrt/include;/work/sysroot/windows-sdk/include/shared;/work/sysroot/windows-sdk/include/um;/work/sysroot/windows-sdk/include/winrt'
export LIB='/work/sysroot/msvc/lib/x64;/work/sysroot/ucrt/lib/x64;/work/sysroot/windows-sdk/lib/um/x64'
export SOURCE_DATE_EPOCH='1787875200'
export TZ='UTC'
export LC_ALL='C.UTF-8'

for tool in clang-cl lld-link llvm-lib llvm-rc llvm-mt nasm pkg-config zstd; do
  command -v "$tool"
done
"${MESON[@]}" --version
nasm -v
pkg-config --version

meson_project() {
  local name="$1"
  local source="$2"
  shift 2
  "${MESON[@]}" setup "$MESON_BUILD_ROOT/build-$name" "$source" \
    --cross-file="$CROSS" \
    --native-file="$NATIVE_FILE" \
    --prefix="$PREFIX" \
    --libdir=lib \
    --buildtype=release \
    --default-library=static \
    --wrap-mode=nodownload \
    "$@"
  "${MESON[@]}" compile -C "$MESON_BUILD_ROOT/build-$name" -j "$JOBS"
  "${MESON[@]}" install -C "$MESON_BUILD_ROOT/build-$name"
}

meson_project freetype "$SOURCES/freetype" \
  -Dzlib=disabled -Dbzip2=disabled -Dpng=disabled -Dharfbuzz=disabled \
  -Dbrotli=disabled -Dtests=disabled
meson_project fribidi "$SOURCES/fribidi" -Ddocs=false -Dbin=false -Dtests=false
meson_project harfbuzz "$SOURCES/harfbuzz" \
  -Dfreetype=enabled -Dglib=disabled -Dgobject=disabled -Dcairo=disabled \
  -Dicu=disabled -Dtests=disabled -Ddocs=disabled -Dbenchmark=disabled \
  -Dutilities=disabled
meson_project libass "$SOURCES/libass" \
  -Dfontconfig=disabled -Ddirectwrite=enabled -Dasm=enabled -Dlibunibreak=disabled \
  -Dtest=disabled -Dcompare=disabled -Dprofile=disabled -Dfuzz=disabled \
  -Dcheckasm=disabled
meson_project lcms2 "$SOURCES/lcms2" -Dfastfloat=true -Dthreaded=true -Djpeg=disabled -Dtiff=disabled

cmake -S "$SOURCES/spirv-cross" -B /work/build-spirv-cross -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_TOOLCHAIN_FILE="$CMAKE_TOOLCHAIN" \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DCMAKE_POLICY_DEFAULT_CMP0091=NEW \
  -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded \
  -DBUILD_SHARED_LIBS=OFF \
  -DSPIRV_CROSS_SHARED=ON \
  -DSPIRV_CROSS_CLI=OFF \
  -DSPIRV_CROSS_ENABLE_TESTS=OFF \
  -DSPIRV_CROSS_ENABLE_MSL=OFF \
  -DSPIRV_CROSS_ENABLE_CPP=OFF \
  -DSPIRV_CROSS_ENABLE_REFLECT=OFF \
  -DSPIRV_CROSS_ENABLE_UTIL=OFF
ninja -C /work/build-spirv-cross -j "$JOBS"
ninja -C /work/build-spirv-cross -j "$JOBS" install

cmake -S "$SOURCES/shaderc" -B /work/build-shaderc -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_TOOLCHAIN_FILE="$CMAKE_TOOLCHAIN" \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DCMAKE_POLICY_DEFAULT_CMP0091=NEW \
  -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded \
  -DSHADERC_SKIP_TESTS=ON \
  -DSHADERC_SKIP_EXAMPLES=ON \
  -DSHADERC_SKIP_COPYRIGHT_CHECK=ON \
  -DSHADERC_SKIP_INSTALL=ON \
  -DGLSLANG_ENABLE_INSTALL=OFF \
  -DSPIRV_SKIP_EXECUTABLES=ON \
  -DSPIRV_SKIP_TESTS=ON \
  -DENABLE_GLSLANG_BINARIES=OFF \
  -DSPIRV_TOOLS_BUILD_STATIC=ON
ninja -C /work/build-shaderc -j "$JOBS" shaderc_combined
install -d "$PREFIX/include" "$PREFIX/lib/pkgconfig"
cp -R "$SOURCES/shaderc/libshaderc/include/shaderc" "$PREFIX/include/"
install -m 0644 /work/build-shaderc/libshaderc/shaderc_combined.lib "$PREFIX/lib/shaderc_combined.lib"
printf '%s\n' \
  "prefix=$PREFIX" \
  'libdir=${prefix}/lib' \
  'includedir=${prefix}/include' \
  'Name: shaderc' \
  'Description: Static shaderc combined library' \
  'Version: 2026.1' \
  'Libs: -L${libdir} -lshaderc_combined' \
  'Cflags: -I${includedir}' > "$PREFIX/lib/pkgconfig/shaderc.pc"

cmake -S "$SOURCES/vulkan-headers" -B /work/build-vulkan-headers -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_TOOLCHAIN_FILE="$CMAKE_TOOLCHAIN" \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DVULKAN_HEADERS_ENABLE_MODULE=OFF
ninja -C /work/build-vulkan-headers -j "$JOBS" install

cmake -S "$SOURCES/vulkan-loader" -B /work/build-vulkan-loader -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_TOOLCHAIN_FILE="$CMAKE_TOOLCHAIN" \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DVULKAN_HEADERS_INSTALL_DIR="$PREFIX" \
  -DUPDATE_DEPS=OFF \
  -DBUILD_TESTS=OFF \
  -DENABLE_WERROR=OFF
ninja -C /work/build-vulkan-loader -j "$JOBS"
ninja -C /work/build-vulkan-loader -j "$JOBS" install

pushd "$SOURCES/ffmpeg" >/dev/null
./configure \
  --prefix="$PREFIX" \
  --target-os=win32 \
  --arch=x86_64 \
  --toolchain=msvc \
  --enable-cross-compile \
  --cc=clang-cl \
  --cxx=clang-cl \
  --ld=lld-link \
  --ar=llvm-lib \
  --ranlib=true \
  --nm=llvm-nm \
  --strip=llvm-strip \
  --windres=llvm-rc \
  --pkg-config="$PKG_CONFIG" \
  --enable-static \
  --disable-shared \
  --disable-autodetect \
  --enable-gpl \
  --enable-d3d11va \
  --enable-hwaccel=av1_d3d11va \
  --enable-hwaccel=av1_d3d11va2 \
  --enable-hwaccel=h264_d3d11va \
  --enable-hwaccel=h264_d3d11va2 \
  --enable-hwaccel=hevc_d3d11va \
  --enable-hwaccel=hevc_d3d11va2 \
  --enable-hwaccel=mpeg2_d3d11va \
  --enable-hwaccel=mpeg2_d3d11va2 \
  --enable-hwaccel=vc1_d3d11va \
  --enable-hwaccel=vc1_d3d11va2 \
  --enable-hwaccel=vp9_d3d11va \
  --enable-hwaccel=vp9_d3d11va2 \
  --enable-hwaccel=wmv3_d3d11va \
  --enable-hwaccel=wmv3_d3d11va2 \
  --disable-debug \
  --disable-doc \
  --disable-network \
  --disable-programs \
  --extra-cflags='/MT /Brepro /clang:-ffile-prefix-map=/work=/usr/src/autolive-mpv /imsvc/work/sysroot/msvc/include /imsvc/work/sysroot/ucrt/include /imsvc/work/sysroot/windows-sdk/include/shared /imsvc/work/sysroot/windows-sdk/include/um /imsvc/work/sysroot/windows-sdk/include/winrt' \
  --extra-ldflags='/Brepro /libpath:/work/sysroot/msvc/lib/x64 /libpath:/work/sysroot/ucrt/lib/x64 /libpath:/work/sysroot/windows-sdk/lib/um/x64'
make -j"$JOBS"
make install
popd >/dev/null

meson_project libplacebo "$SOURCES/libplacebo" \
  -Dvulkan=enabled -Dvk-proc-addr=enabled -Dd3d11=enabled -Dshaderc=enabled \
  -Dglslang=disabled -Dlcms=enabled -Dopengl=disabled -Dlibdovi=disabled \
  -Dxxhash=disabled -Dunwind=disabled -Ddemos=false -Dtests=false

"${MESON[@]}" setup "$MESON_BUILD_ROOT/build-mpv" "$SOURCES/mpv" \
  --cross-file="$CROSS" \
  --native-file="$NATIVE_FILE" \
  --prefix="$PREFIX" \
  --libdir=lib \
  --buildtype=release \
  --wrap-mode=nodownload \
  -Dauto_features=disabled \
  -Dbuild-date=false \
  -Dgl=disabled \
  -Ddefault_library=static \
  -Dfuzzers=false \
  -Dtests=false \
  -Dcplayer=true \
  -Dd3d11=enabled \
  -Dd3d-hwaccel=enabled \
  -Dlcms2=enabled \
  -Dshaderc=enabled \
  -Dspirv-cross=enabled \
  -Dvulkan=enabled \
  -Dwin32-threads=enabled \
  -Dcdda=disabled \
  -Dcplugins=disabled \
  -Ddvbin=disabled \
  -Ddvdnav=disabled \
  -Djavascript=disabled \
  -Dlibarchive=disabled \
  -Dlibavdevice=disabled \
  -Dlibbluray=disabled \
  -Dlibcurl=disabled \
  -Dlibmpv=false \
  -Dlua=disabled \
  -Dmanpage-build=disabled \
  -Dopenal=disabled \
  -Dsdl2-audio=disabled \
  -Dsdl2-gamepad=disabled \
  -Dsdl2-video=disabled \
  -Dsixel=disabled \
  -Dsubrandr=disabled \
  -Dvapoursynth=disabled \
  -Dwasapi=disabled
ninja -C "$MESON_BUILD_ROOT/build-mpv" -j "$JOBS" mpv.exe

test -s "$MESON_BUILD_ROOT/build-mpv/mpv.exe"
test -s "$PREFIX/bin/spirv-cross-c-shared.dll"
test -s "$PREFIX/bin/vulkan-1.dll"
install -m 0644 "$MESON_BUILD_ROOT/build-mpv/mpv.exe" /work/mpv.exe
install -m 0644 "$PREFIX/bin/spirv-cross-c-shared.dll" /work/spirv-cross-c-shared.dll
install -m 0644 "$PREFIX/bin/vulkan-1.dll" /work/vulkan-1.dll
