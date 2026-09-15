#include <cassert>
#include <cstddef>
#include <cstdint>
#include <vector>
#include "yuy2_black_frame.h"

int main() {
    // Two distinct old pictures must become the same neutral limited-range
    // frame. Guard bytes prove no bytes outside the frame are modified.
    for (const std::size_t bytes : {1280u * 720u * 2u, 1920u * 1080u * 2u,
                                  1080u * 1920u * 2u}) {
        for (const auto previous : {0x00, 0xff}) {
            std::vector<std::uint8_t> frame(bytes + 2, static_cast<std::uint8_t>(previous));
            AkVCam::fillYuy2BlackFrame(frame.data() + 1, bytes);
            assert(frame.front() == previous && frame.back() == previous);
            for (std::size_t index = 0; index < bytes; index += 4) {
                assert(frame[index + 1] == 16 && frame[index + 2] == 128);
                assert(frame[index + 3] == 16 && frame[index + 4] == 128);
            }
        }
    }
}
