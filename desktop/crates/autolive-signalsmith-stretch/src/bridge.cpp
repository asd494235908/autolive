#include "bridge.h"
#include <signalsmith-stretch/signalsmith-stretch.h>

#include <memory>

template<typename Sample>
class InterleavedBuffer {
    Sample *data;
    int channels;

public:
    InterleavedBuffer(Sample *data, int channels) : data(data), channels(channels) {}

    class ChannelView {
        Sample *data;
        int channel;
        int stride;

    public:
        ChannelView(Sample *data, int channel, int stride)
            : data(data), channel(channel), stride(stride) {}
        Sample &operator[](size_t frame) { return data[frame * stride + channel]; }
        const Sample &operator[](size_t frame) const { return data[frame * stride + channel]; }
    };

    ChannelView operator[](size_t channel) {
        return ChannelView(data, static_cast<int>(channel), channels);
    }
};

struct autolive_stretch {
    signalsmith::stretch::SignalsmithStretch<float> instance{0};
    int channels;
};

autolive_stretch_t *autolive_stretch_create(
    int channels,
    float sample_rate_hz,
    float pitch_semitones,
    float formant_factor,
    float formant_base) {
    try {
        auto handle = std::make_unique<autolive_stretch>();
        handle->channels = channels;
        handle->instance.presetDefault(channels, sample_rate_hz);
        handle->instance.setTransposeSemitones(pitch_semitones);
        handle->instance.setFormantFactor(formant_factor, true);
        handle->instance.setFormantBase(formant_base);
        return handle.release();
    } catch (...) {
        return nullptr;
    }
}

void autolive_stretch_destroy(autolive_stretch_t *handle) {
    delete handle;
}

size_t autolive_stretch_input_latency(const autolive_stretch_t *handle) {
    return static_cast<size_t>(handle->instance.inputLatency());
}

size_t autolive_stretch_output_latency(const autolive_stretch_t *handle) {
    return static_cast<size_t>(handle->instance.outputLatency());
}

bool autolive_stretch_process(
    autolive_stretch_t *handle,
    const float *input,
    size_t frames,
    float *output) {
    try {
        InterleavedBuffer<const float> input_buffer(input, handle->channels);
        InterleavedBuffer<float> output_buffer(output, handle->channels);
        handle->instance.process(input_buffer, frames, output_buffer, frames);
        return true;
    } catch (...) {
        return false;
    }
}

bool autolive_stretch_flush(autolive_stretch_t *handle, float *output, size_t frames) {
    try {
        InterleavedBuffer<float> output_buffer(output, handle->channels);
        handle->instance.flush(output_buffer, frames);
        return true;
    } catch (...) {
        return false;
    }
}

bool autolive_stretch_reset(autolive_stretch_t *handle) {
    try {
        handle->instance.reset();
        return true;
    } catch (...) {
        return false;
    }
}
