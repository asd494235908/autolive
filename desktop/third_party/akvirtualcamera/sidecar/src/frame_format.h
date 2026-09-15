#pragma once
#include <cstdint>

// Pure protocol boundary, shared with the native test (no Windows dependency).
inline bool validFrameFormat(std::uint16_t version, std::uint32_t width,
                             std::uint32_t height, std::uint32_t bytes) {
    return (version == 1 || version == 2)
        && (version != 1 || (width == 1280 && height == 720))
        && width > 0 && height > 0 && width <= 4096 && height <= 4096
        && width % 2 == 0 && static_cast<std::uint64_t>(width) * height <= 4096u * 2160u
        && bytes == static_cast<std::uint64_t>(width) * height * 2u;
}
