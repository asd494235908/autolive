#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "$0")/.." && pwd)"
contract="$root_dir/媒体参数范围与默认值.md"
shared_types="$root_dir/接口契约/共享类型.md"
test -f "$contract" || { echo "缺少媒体参数契约" >&2; exit 1; }
test -f "$shared_types" || { echo "缺少共享类型契约" >&2; exit 1; }

required_patterns=(
  '随机变声周期'
  'MFCC 偏移'
  '当前 SNR'
  '当前共振峰'
  '滤波 Q 值'
  '下一次变声'
  '帧率微扰频率'
  '帧率微扰幅度'
  '65Hz～20000Hz'
  'band_weight'
  'SHA-256 阶段哈希契约'
)

for pattern in "${required_patterns[@]}"; do
  rg -q --fixed-strings "$pattern" "$contract" || {
    echo "参数契约缺少字段: $pattern" >&2
    exit 1
  }
done

for pattern in 'SpeechToSpeechContext' 'SpeechToSpeechResult' 'AudioVariantCandidate'; do
  rg -q --fixed-strings "$pattern" "$shared_types" || {
    echo "共享类型契约缺少字段: $pattern" >&2
    exit 1
  }
done

echo "媒体参数契约检查通过"
