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
