#ifndef AUTOLIVE_SIGNALSMITH_STRETCH_BRIDGE_H
#define AUTOLIVE_SIGNALSMITH_STRETCH_BRIDGE_H

#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct autolive_stretch autolive_stretch_t;

autolive_stretch_t *autolive_stretch_create(
    int channels,
    float sample_rate_hz,
    float pitch_semitones,
    float formant_factor,
    float formant_base);
void autolive_stretch_destroy(autolive_stretch_t *handle);
size_t autolive_stretch_input_latency(const autolive_stretch_t *handle);
size_t autolive_stretch_output_latency(const autolive_stretch_t *handle);
bool autolive_stretch_process(
    autolive_stretch_t *handle,
    const float *input,
    size_t frames,
    float *output);
bool autolive_stretch_flush(autolive_stretch_t *handle, float *output, size_t frames);
bool autolive_stretch_reset(autolive_stretch_t *handle);

#ifdef __cplusplus
}
#endif

#endif
