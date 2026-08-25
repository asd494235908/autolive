# Third-party notices

- Signalsmith Stretch 1.3.1, copyright Signalsmith Audio Ltd., MIT license.
- Signalsmith Linear (commit `157b448` as bundled by `signalsmith-stretch` 0.1.3), copyright Signalsmith Audio, MIT license.

The unmodified upstream headers and their license texts are stored under `vendor/`.

The native adapter is compiled at optimization level 2 in every Rust profile because upstream
documents substantially slower unoptimized processing. Fast-math is deliberately not enabled.
