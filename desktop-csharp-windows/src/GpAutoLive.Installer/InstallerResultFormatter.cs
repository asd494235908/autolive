using System.Text.Json;
using System.Text.RegularExpressions;

namespace GpAutoLive.Installer;

internal sealed record InstallerResultSummary(bool IsHealthy, string Text);

internal static partial class InstallerResultFormatter
{
    public static InstallerResultSummary Format(
        InstallerOperation operation,
        InstallerScriptResult result)
    {
        ArgumentNullException.ThrowIfNull(result);
        if (result.Cancelled)
        {
            return new(false, "操作已取消。请刷新安装状态，确认当前活动版本和临时 staging 状态。 ");
        }
        if (result.TimedOut)
        {
            return new(false, "操作超时，已终止安装维护进程。请刷新安装状态后重试。 ");
        }
        if (!result.ProcessSucceeded)
        {
            return new(false, FormatProcessFailure(result));
        }

        try
        {
            using var document = JsonDocument.Parse(result.StandardOutput);
            var root = document.RootElement;
            var status = GetString(root, "status");
            var code = GetString(root, "code");
            var lines = new List<string>();
            if (status is not null)
            {
                lines.Add($"状态：{status}");
            }
            if (code is not null)
            {
                lines.Add($"代码：{code}");
            }

            if (root.TryGetProperty("active", out var active)
                && active.ValueKind == JsonValueKind.Object)
            {
                var version = GetString(active, "version");
                if (version is not null)
                {
                    lines.Add($"活动版本：{version}");
                }
            }
            AddString(lines, root, "installed_version", "安装版本");
            AddString(lines, root, "active_version", "活动版本");
            AddString(lines, root, "previous_version", "上一版本");
            AddString(lines, root, "rollback_version", "回滚版本");
            AddString(lines, root, "uninstall_version", "删除版本");

            if (root.TryGetProperty("rollback", out var rollback)
                && rollback.ValueKind == JsonValueKind.Object
                && rollback.TryGetProperty("available", out var available)
                && available.ValueKind is JsonValueKind.True or JsonValueKind.False)
            {
                lines.Add(available.GetBoolean()
                    ? $"可回滚版本：{GetString(rollback, "version") ?? "已验证"}"
                    : "可回滚版本：无");
            }

            if (root.TryGetProperty("versions", out var versions)
                && versions.ValueKind == JsonValueKind.Array)
            {
                var names = versions.EnumerateArray()
                    .Select(item => GetString(item, "version"))
                    .Where(value => value is not null)
                    .Take(100);
                lines.Add($"已安装版本：{string.Join(", ", names!)}");
            }

            var healthy = IsHealthyStatus(operation, root, status);
            if (lines.Count == 0)
            {
                lines.Add(operation == InstallerOperation.VerifyPackage
                    ? "发布包校验通过。"
                    : "脚本执行完成。 ");
            }
            if (operation == InstallerOperation.CheckRuntime && status == "runtime_missing")
            {
                lines.Add("需要安装 .NET 10 Windows Desktop Runtime；本维护壳不会自动下载。 ");
            }
            return new(healthy, string.Join(Environment.NewLine, lines));
        }
        catch (Exception exception) when (exception is JsonException or InvalidOperationException)
        {
            return new(false, "脚本返回了无法识别的结果；未据此判定操作成功。 ");
        }
    }

    private static bool IsHealthyStatus(
        InstallerOperation operation,
        JsonElement root,
        string? status) =>
        operation switch
        {
            InstallerOperation.VerifyPackage => IsVerifiedPackageReport(root),
            InstallerOperation.CheckInstallState => IsHealthyInstallState(root, status),
            InstallerOperation.CheckRuntime => status is "ready" or "installer_ready" or "installed" or "installed_reboot_required",
            InstallerOperation.InstallOrUpgrade => status is "what_if" or "activated",
            InstallerOperation.Rollback => status is "what_if" or "rolled_back",
            InstallerOperation.UninstallVersion => status is "what_if" or "uninstalled",
            _ => false
        };

    private static bool IsHealthyInstallState(JsonElement root, string? status)
    {
        if (status is not "healthy"
            || !root.TryGetProperty("schema_version", out var schema)
            || schema.ValueKind != JsonValueKind.Number
            || !schema.TryGetInt32(out var schemaVersion)
            || schemaVersion != 1
            || !root.TryGetProperty("active", out var active)
            || active.ValueKind != JsonValueKind.Object
            || !TryGetRequiredString(active, "version", out var activeVersion)
            || !root.TryGetProperty("rollback", out var rollback)
            || rollback.ValueKind != JsonValueKind.Object
            || !rollback.TryGetProperty("available", out var rollbackAvailable)
            || rollbackAvailable.ValueKind is not (JsonValueKind.True or JsonValueKind.False)
            || !root.TryGetProperty("versions", out var versions)
            || versions.ValueKind != JsonValueKind.Array
            || versions.GetArrayLength() == 0)
        {
            return false;
        }

        var activeMatches = 0;
        var rollbackMatches = 0;
        foreach (var version in versions.EnumerateArray())
        {
            if (version.ValueKind != JsonValueKind.Object
                || !TryGetRequiredString(version, "version", out var versionName)
                || !TryGetRequiredString(version, "status", out var versionStatus)
                || versionStatus is not "verified"
                || !version.TryGetProperty("active", out var isActive)
                || isActive.ValueKind is not (JsonValueKind.True or JsonValueKind.False)
                || !version.TryGetProperty("rollback_candidate", out var isRollback)
                || isRollback.ValueKind is not (JsonValueKind.True or JsonValueKind.False))
            {
                return false;
            }

            if (isActive.GetBoolean() && string.Equals(versionName, activeVersion, StringComparison.Ordinal))
            {
                activeMatches++;
            }

            if (isRollback.GetBoolean())
            {
                if (!rollbackAvailable.GetBoolean()
                    || !rollback.TryGetProperty("version", out var rollbackVersion)
                    || rollbackVersion.ValueKind != JsonValueKind.String
                    || !string.Equals(versionName, rollbackVersion.GetString(), StringComparison.Ordinal))
                {
                    return false;
                }
                rollbackMatches++;
            }
        }

        if (activeMatches != 1)
        {
            return false;
        }

        if (!rollbackAvailable.GetBoolean())
        {
            return rollbackMatches == 0
                && (!rollback.TryGetProperty("version", out var absentVersion)
                    || absentVersion.ValueKind == JsonValueKind.Null);
        }

        return rollbackMatches == 1;
    }

    private static bool TryGetRequiredString(JsonElement element, string propertyName, out string value)
    {
        value = string.Empty;
        if (!element.TryGetProperty(propertyName, out var property)
            || property.ValueKind != JsonValueKind.String)
        {
            return false;
        }

        value = property.GetString() ?? string.Empty;
        return !string.IsNullOrWhiteSpace(value);
    }

    private static bool IsVerifiedPackageReport(JsonElement root)
    {
        if (!root.TryGetProperty("schema_version", out var schema)
            || schema.ValueKind != JsonValueKind.Number
            || !schema.TryGetInt32(out var schemaVersion)
            || schemaVersion != 1)
        {
            return false;
        }

        string[] requiredGroups =
        [
            "package_root_files",
            "gpu_manifest_files",
            "winrt_manifest_files",
            "media_manifest_files"
        ];
        return requiredGroups.All(group =>
            root.TryGetProperty(group, out var files)
            && files.ValueKind == JsonValueKind.Array
            && files.GetArrayLength() > 0);
    }

    private static string FormatProcessFailure(InstallerScriptResult result)
    {
        var code = result.StandardError.Trim();
        if (code == "powershell_7_required")
        {
            return "未找到可用的 PowerShell 7。此维护壳当前要求 .NET 10 Desktop Runtime 与 PowerShell 7。 ";
        }
        if (code == "output_limit")
        {
            return "脚本输出超过 256 KiB 安全上限，操作已终止。 ";
        }

        var redacted = Redact(code);
        return string.IsNullOrWhiteSpace(redacted)
            ? $"脚本执行失败（退出码 {result.ExitCode}）。"
            : $"脚本执行失败（退出码 {result.ExitCode}）：{redacted}";
    }

    internal static string Redact(string value)
    {
        var bounded = value.Length > 2000 ? value[^2000..] : value;
        bounded = UncPathPattern().Replace(bounded, "<网络路径>");
        bounded = WindowsPathPattern().Replace(bounded, "<本地路径>");
        return bounded.Trim();
    }

    private static void AddString(
        List<string> lines,
        JsonElement root,
        string propertyName,
        string label)
    {
        var value = GetString(root, propertyName);
        if (value is not null)
        {
            lines.Add($"{label}：{value}");
        }
    }

    private static string? GetString(JsonElement element, string propertyName)
    {
        if (!element.TryGetProperty(propertyName, out var property)
            || property.ValueKind is JsonValueKind.Null or JsonValueKind.Undefined)
        {
            return null;
        }
        return property.ValueKind == JsonValueKind.String
            ? property.GetString()
            : property.ToString();
    }

    [GeneratedRegex("""(?i)\b[A-Z]:\\.*?(?=\s+(?:and|at|in|from|to)\s+|[\r\n"']|$)""")]
    private static partial Regex WindowsPathPattern();

    [GeneratedRegex("""\\\\[^\r\n"']+""")]
    private static partial Regex UncPathPattern();
}
