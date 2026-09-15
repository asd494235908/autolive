# GPL-3.0-or-later independent GpAutoLive sidecar.
cmake_minimum_required(VERSION 3.16)

if (NOT WIN32)
    message(FATAL_ERROR "GpAutoLive AkVirtualCamera sidecar is Windows-only")
endif ()

project(AkVirtualCameraSidecar LANGUAGES CXX)

add_executable(AkVirtualCameraSidecar src/main.cpp)
target_compile_features(AkVirtualCameraSidecar PRIVATE cxx_std_17)
target_compile_definitions(AkVirtualCameraSidecar PRIVATE UNICODE _UNICODE NOMINMAX)
if (MSVC)
    target_compile_options(AkVirtualCameraSidecar PRIVATE /utf-8)
endif ()
target_link_libraries(AkVirtualCameraSidecar PRIVATE advapi32)
set_target_properties(AkVirtualCameraSidecar PROPERTIES
                      OUTPUT_NAME akvirtualcamera-sidecar-x64
                      WIN32_EXECUTABLE FALSE)

install(TARGETS AkVirtualCameraSidecar RUNTIME DESTINATION ${BINDIR})

add_executable(AkVirtualCameraFrameFormatTest src/frame_format_test.cpp)
target_compile_features(AkVirtualCameraFrameFormatTest PRIVATE cxx_std_17)
if (MSVC)
    target_compile_options(AkVirtualCameraFrameFormatTest PRIVATE /UNDEBUG)
endif ()
add_executable(AkVirtualCameraDeviceFormatTest src/device_format_test.cpp)
target_compile_features(AkVirtualCameraDeviceFormatTest PRIVATE cxx_std_17)
target_link_libraries(AkVirtualCameraDeviceFormatTest PRIVATE advapi32)
if (MSVC)
    target_compile_options(AkVirtualCameraDeviceFormatTest PRIVATE /UNDEBUG)
endif ()

add_executable(AkVirtualCameraBlackFrameTest src/yuy2_black_frame_test.cpp)
target_compile_features(AkVirtualCameraBlackFrameTest PRIVATE cxx_std_17)
target_include_directories(AkVirtualCameraBlackFrameTest PRIVATE
    "${CMAKE_SOURCE_DIR}/windows/dshow/BaseFilter/src")
if (MSVC)
    target_compile_options(AkVirtualCameraBlackFrameTest PRIVATE /UNDEBUG)
endif ()

add_executable(AkVirtualCameraYuy2SampleTest src/yuy2_sample_test.cpp)
target_compile_features(AkVirtualCameraYuy2SampleTest PRIVATE cxx_std_20)
target_include_directories(AkVirtualCameraYuy2SampleTest PRIVATE
    "${CMAKE_SOURCE_DIR}" "${CMAKE_SOURCE_DIR}/windows")
target_link_libraries(AkVirtualCameraYuy2SampleTest PRIVATE PlatformUtils_windows ole32)
target_compile_definitions(AkVirtualCameraYuy2SampleTest PRIVATE NOMINMAX)
