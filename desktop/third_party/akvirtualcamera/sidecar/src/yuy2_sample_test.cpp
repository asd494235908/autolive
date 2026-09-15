#include <windows.h>
#include <dshow.h>
#include <cstdio>
#include "PlatformUtils/src/utils.h"
#include "VCamUtils/src/videoformat.h"
#include "VCamUtils/src/fraction.h"
#include "VCamUtils/src/videoframe.h"
#include "PlatformUtils/src/yuy2_sample.h"

int main() {
    // Actual upstream media-type builder, including an odd-height source.
    for (const auto dimensions : {std::pair<int, int>{1080, 1920}, {1080, 1919},
                                  {1280, 720}, {1920, 1080}}) {
        AkVCam::VideoFormat format(AkVCam::PixelFormat_yuyv422,
                                   dimensions.first, dimensions.second, {30, 1});
        auto *media = AkVCam::mediaTypeFromFormat(format);
        const auto bytes = static_cast<ULONG>(dimensions.first * dimensions.second * 2);
        if (!media) return 1;
        auto *info = reinterpret_cast<VIDEOINFOHEADER *>(media->pbFormat);
        const bool correct = media->lSampleSize == bytes && info->bmiHeader.biSizeImage == bytes;
        std::fprintf(stdout, "%dx%d: media=%lu bitmap=%lu expected=%lu\n", dimensions.first,
                     dimensions.second, media->lSampleSize, info->bmiHeader.biSizeImage, bytes);
        CoTaskMemFree(media->pbFormat); CoTaskMemFree(media);
        if (!correct) return 1;

        AkVCam::VideoFrame frame(format);
        std::memset(frame.data(), 0xef, frame.size());
        const auto rowBytes = static_cast<std::size_t>(dimensions.first) * 2;
        for (int row = 0; row < dimensions.second; ++row)
            for (std::size_t column = 0; column < rowBytes; ++column)
                frame.line(0, row)[column] = static_cast<std::uint8_t>(row * 3 + column * 7);
        std::vector<std::uint8_t> sample(bytes + 2, 0xad);
        if (!AkVCam::copyPackedYuy2Sample(sample.data() + 1, bytes, frame.constData(),
                                        frame.lineSize(0), dimensions.first, dimensions.second)) return 1;
        if (sample.front() != 0xad || sample.back() != 0xad) return 1;
        for (int row = 0; row < dimensions.second; ++row)
            if (std::memcmp(sample.data() + 1 + row * rowBytes, frame.constLine(0, row), rowBytes)) {
                std::fprintf(stderr, "YUY2 row padding leaked at row %d\n", row);
                return 1;
            }
        if (AkVCam::copyPackedYuy2Sample(sample.data(), bytes - 1, frame.constData(),
                                       frame.lineSize(0), dimensions.first, dimensions.second)) return 1;
    }
    return 0;
}
