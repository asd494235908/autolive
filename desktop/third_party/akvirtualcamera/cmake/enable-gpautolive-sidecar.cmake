# Loaded through CMAKE_PROJECT_INCLUDE during the local, offline AkVirtualCamera
# build. The guard prevents nested upstream project() calls from adding the
# sidecar more than once.
if (PROJECT_NAME STREQUAL "AkVirtualCamera"
    AND WIN32
    AND AKVCAM_BUILD_GPAUTOLIVE_SIDECAR
    AND (CMAKE_GENERATOR_PLATFORM STREQUAL "x64" OR CMAKE_SIZEOF_VOID_P EQUAL 8)
    AND NOT TARGET AkVirtualCameraSidecar)
    add_subdirectory(
        "${AKVCAM_GPAUTOLIVE_SIDECAR_SOURCE}"
        "${CMAKE_BINARY_DIR}/gpautolive-sidecar")
endif ()
