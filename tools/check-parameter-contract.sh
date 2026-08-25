#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "$0")/.." && pwd)"
contract="$root_dir/媒体参数范围与默认值.md"
test -f "$contract" || { echo "缺少媒体参数契约" >&2; exit 1; }

current_video_section="$(sed -n '/^### 4\.1 /,/^### 4\.2 /p' "$contract")"

required_video_patterns=(
  '亮度'
  '对比度'
  '饱和度'
  '色相旋转'
  '模糊半径'
  '锐化'
  '噪点强度'
  '细节增强'
  '动态裁剪'
  'X/Y 空间偏移'
  '像素扰动'
  '像素级缩放'
  '帧率扰动'
  '帧内/帧间扰动'
)

for pattern in "${required_video_patterns[@]}"; do
  printf '%s' "$current_video_section" | rg -q --fixed-strings "$pattern" || {
    echo "当前视频参数契约缺少字段: $pattern" >&2
    exit 1
  }
done

if printf '%s' "$current_video_section" | rg -q '研究参数|实验参数'; then
  echo "当前视频参数表仍使用已废弃的研究或实验命名" >&2
  exit 1
fi

for pattern in '输入增益' '输出增益' '响度' '三段 EQ' '音高' '播放速度' '降噪' '轻混响' '采样率' '输出码率'; do
  rg -q --fixed-strings "$pattern" "$contract" || {
    echo "普通声音参数契约缺少字段: $pattern" >&2
    exit 1
  }
done

for pattern in '本版本不开发实时话术幻化' '参考图中的音频、视频、高级视觉、挂件、切片、检测和修复参数全部属于正式产品需求' '65–20000Hz 明确是视频空间频段'; do
  rg -q --fixed-strings "$pattern" "$contract" || {
    echo "参数契约缺少当前范围声明: $pattern" >&2
    exit 1
  }
done

echo "媒体参数契约检查通过"
