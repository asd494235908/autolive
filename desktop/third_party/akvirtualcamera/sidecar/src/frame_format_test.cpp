#include "frame_format.h"
#include <cassert>
int main() {
    assert(validFrameFormat(1, 1280, 720, 1843200));
    assert(!validFrameFormat(1, 1920, 1080, 4147200));
    assert(validFrameFormat(2, 1920, 1080, 4147200));
    assert(validFrameFormat(2, 1080, 1920, 4147200));
    assert(validFrameFormat(2, 4096, 2160, 17694720));
    assert(!validFrameFormat(2, 4096, 4096, 33554432));
    assert(!validFrameFormat(2, 1921, 1080, 4149360));
    assert(!validFrameFormat(2, 1920, 1080, 1843200));
    assert(!validFrameFormat(2, 0, 720, 0));
    assert(!validFrameFormat(3, 1280, 720, 1843200));
}
