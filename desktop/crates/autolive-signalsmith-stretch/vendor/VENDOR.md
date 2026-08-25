# Signalsmith vendor record

- Source package: crates.io `signalsmith-stretch` 0.1.3, package VCS commit
  `c4d0cbd41351966f7fc0b18b9d83d9e381aa4e26`.
- Upstream libraries: Signalsmith Stretch 1.3.1 (`c598726`) and Signalsmith Linear
  (`157b448`) as bundled by that package.
- License: MIT; the complete license texts are stored beside each library.
- Copied files are unmodified. SHA-256:
  - `signalsmith-stretch/signalsmith-stretch.h`:
    `A1AD98EAAF81380723B5F39EF5389FA5AA27FA62C11C6173701431F96510A786`
  - `signalsmith-linear/stft.h`:
    `727E19C35EE1792BF72863F07D96D78A4B3E9669E35374ADE5AA523C667988CF`
  - `signalsmith-linear/fft.h`:
    `FB3409A88CDD4354BAFE3286EF38C673F26EDEBD36BF0D4D002A2ACECC1E6037`

Only the headers required by the adapter are vendored. Updating means replacing these pinned files,
updating the hashes and notices, then rerunning the crate tests and the desktop workspace gates.
Removing the feature means deleting this crate and its single path dependency/call site.

The native adapter is compiled locally through the pinned `cc` build dependency at optimization
level 2. It does not enable fast-math and does not require bindgen or libclang.
