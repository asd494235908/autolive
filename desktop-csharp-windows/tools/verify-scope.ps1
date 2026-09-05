[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

# C# 端只允许在 desktop-csharp-windows/ 下演进；此门禁检查已跟踪 Rust/Tauri 文件是否被改动。
$changedTrackedDesktopFiles = @(git -C $repoRoot diff --name-only HEAD -- desktop/ 2>$null)
$changedUntrackedDesktopFiles = @(git -C $repoRoot ls-files --others --exclude-standard -- desktop/ 2>$null)
$changedDesktopFiles = @($changedTrackedDesktopFiles + $changedUntrackedDesktopFiles | Sort-Object -Unique)
if ($changedDesktopFiles.Count -gt 0)
{
    Write-Output "Scope violation: found $($changedDesktopFiles.Count) Rust/Tauri path(s)."
    $changedDesktopFiles | Select-Object -First 20 | ForEach-Object { Write-Output "Scope violation: $_" }
    if ($changedDesktopFiles.Count -gt 20)
    {
        Write-Output 'Scope violation: output truncated; inspect git status for the complete list.'
    }
    exit 1
}

Write-Output 'Scope OK: tracked desktop/ files are unchanged.'
