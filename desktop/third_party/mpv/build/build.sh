#!/usr/bin/env bash
set -Eeuo pipefail

readonly TARGET='x86_64-pc-windows-msvc'
readonly INPUT_ROOT='/build-inputs'
readonly OUTPUT_ROOT='/out'
readonly REQUIRED_INPUTS='/recipe/required-inputs.txt'

blockers=()

block() {
  blockers+=("$1")
}

if [[ $# -ne 0 ]]; then
  block 'arguments-not-allowed'
fi

if [[ ! -r /proc/net/route ]]; then
  block 'network-state-unavailable'
elif awk 'NR > 1 && $2 == "00000000" { found = 1 } END { exit !found }' /proc/net/route; then
  block 'network-route-present-run-with-network-none'
fi

if [[ ! -d "$INPUT_ROOT" ]]; then
  block 'build-inputs-mount-missing'
elif ! awk '$5 == "/build-inputs" && $6 ~ /(^|,)ro(,|$)/ { found = 1 } END { exit !found }' /proc/self/mountinfo; then
  block 'build-inputs-must-be-read-only'
fi

if [[ ! -d "$OUTPUT_ROOT" ]]; then
  block 'output-mount-missing'
elif [[ ! -w "$OUTPUT_ROOT" ]]; then
  block 'output-mount-not-writable'
fi

if [[ -r "$REQUIRED_INPUTS" && -d "$INPUT_ROOT" ]]; then
  while IFS= read -r input || [[ -n "$input" ]]; do
    [[ -z "$input" || "$input" == \#* ]] && continue
    if [[ "$input" == /* || "$input" == *'..'* || ! -f "$INPUT_ROOT/$input" || -L "$INPUT_ROOT/$input" ]]; then
      block "missing-or-invalid-input:$input"
    fi
  done < "$REQUIRED_INPUTS"
else
  block 'required-input-list-unavailable'
fi

if [[ -d "$OUTPUT_ROOT" ]] && find "$OUTPUT_ROOT" -mindepth 1 -maxdepth 1 -print -quit | grep -q .; then
  block 'output-mount-must-be-empty'
fi

if [[ ${#blockers[@]} -gt 0 ]]; then
  printf '{"schemaVersion":1,"gate":"mpv-phase7a-build-recipe","status":"blocked","target":"%s","blockers":[' "$TARGET"
  for index in "${!blockers[@]}"; do
    [[ $index -gt 0 ]] && printf ','
    printf '"%s"' "${blockers[$index]}"
  done
  printf ']}\n'
  exit 78
fi

export SOURCE_DATE_EPOCH='1787875200'
export TZ='UTC'
export LC_ALL='C.UTF-8'

python3 -B /recipe/prepare-inputs.py
set +e
/bin/bash /recipe/build-dependencies.sh 2>&1 | tee /work/build.log
build_status=${PIPESTATUS[0]}
set -e
if [[ $build_status -ne 0 ]]; then
  install -m 0644 /work/build.log "$OUTPUT_ROOT/phase7a-build-failure.log"
  exit "$build_status"
fi

python3 -B /recipe/generate-evidence.py
