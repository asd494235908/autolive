#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root_dir"

required_files=(
  "AGENTS.md"
  "产品需求文档.md"
  "系统架构总览.md"
  "管理系统架构.md"
  "桌面客户端架构.md"
  "媒体参数范围与默认值.md"
  "开发约束/Go开发约束.md"
  "开发约束/Rust开发约束.md"
  "开发约束/前端开发约束.md"
)

for file in "${required_files[@]}"; do
  test -f "$file" || { echo "缺少文档: $file" >&2; exit 1; }
done

if rg -n 'docker\.md|防查重|隐形水印|隐形指纹' --glob '*.md' .; then
  echo "发现失效引用或已废弃名称" >&2
  exit 1
fi

echo "文档引用检查通过"
