using System.IO;
using System.Text.RegularExpressions;

namespace GpAutoLive.Installer;

internal enum InstallerOperation
{
    CheckRuntime,
    VerifyPackage,
    CheckInstallState,
    InstallOrUpgrade,
    Rollback,
    UninstallVersion
}

internal sealed record InstallerRequest(
    InstallerOperation Operation,
    string? PackageRoot,
    string? InstallRoot,
    string? Version,
    bool RequireSigned,
    bool WhatIf);

internal sealed record InstallerScriptInvocation(
    string ScriptName,
    IReadOnlyList<string> Arguments,
    bool IsMutation);

internal static partial class InstallerCommandFactory
{
    private static readonly string[] AllowedInstallRootEntries = ["current.json", "versions"];

    public static InstallerScriptInvocation Create(InstallerRequest request)
    {
        ArgumentNullException.ThrowIfNull(request);

        return request.Operation switch
        {
            InstallerOperation.CheckRuntime => new(
                "bootstrap-csharp-windows-runtime.ps1",
                ["-RequiredMajor", "10"],
                false),
            InstallerOperation.VerifyPackage => CreateVerify(request),
            InstallerOperation.CheckInstallState => CreateState(request),
            InstallerOperation.InstallOrUpgrade => CreateInstall(request),
            InstallerOperation.Rollback => CreateRollback(request),
            InstallerOperation.UninstallVersion => CreateUninstall(request),
            _ => throw new ArgumentOutOfRangeException(nameof(request), "不支持的安装维护操作。")
        };
    }

    private static InstallerScriptInvocation CreateVerify(InstallerRequest request)
    {
        var packageRoot = NormalizeExistingDirectory(request.PackageRoot, "发布包目录");
        var arguments = new List<string> { "-PackageRoot", packageRoot };
        AddRequireSigned(arguments, request.RequireSigned);
        return new("verify-release-package.ps1", arguments, false);
    }

    private static InstallerScriptInvocation CreateState(InstallerRequest request)
    {
        var installRoot = NormalizeInstallRoot(request.InstallRoot, mustExist: true);
        var arguments = new List<string> { "-InstallRoot", installRoot };
        AddWhatIf(arguments, request.WhatIf);
        return new("get-csharp-windows-install-state.ps1", arguments, false);
    }

    private static InstallerScriptInvocation CreateInstall(InstallerRequest request)
    {
        var packageRoot = NormalizeExistingDirectory(request.PackageRoot, "发布包目录");
        var installRoot = NormalizeInstallRoot(request.InstallRoot, mustExist: false);
        AssertSeparateRoots(packageRoot, installRoot);
        var arguments = new List<string>
        {
            "-PackageRoot", packageRoot,
            "-InstallRoot", installRoot,
            "-Version", NormalizeVersion(request.Version, required: true)!
        };
        AddRequireSigned(arguments, request.RequireSigned);
        AddWhatIf(arguments, request.WhatIf);
        arguments.Add("-Confirm:$false");
        return new("install-csharp-windows-package.ps1", arguments, !request.WhatIf);
    }

    private static InstallerScriptInvocation CreateRollback(InstallerRequest request)
    {
        var installRoot = NormalizeInstallRoot(request.InstallRoot, mustExist: true);
        var arguments = new List<string> { "-InstallRoot", installRoot };
        var version = NormalizeVersion(request.Version, required: false);
        if (version is not null)
        {
            arguments.Add("-Version");
            arguments.Add(version);
        }
        AddRequireSigned(arguments, request.RequireSigned);
        AddWhatIf(arguments, request.WhatIf);
        arguments.Add("-Confirm:$false");
        return new("rollback-csharp-windows-package.ps1", arguments, !request.WhatIf);
    }

    private static InstallerScriptInvocation CreateUninstall(InstallerRequest request)
    {
        var installRoot = NormalizeInstallRoot(request.InstallRoot, mustExist: true);
        var arguments = new List<string>
        {
            "-InstallRoot", installRoot,
            "-Version", NormalizeVersion(request.Version, required: true)!
        };
        AddWhatIf(arguments, request.WhatIf);
        arguments.Add("-Confirm:$false");
        return new("uninstall-csharp-windows-package.ps1", arguments, !request.WhatIf);
    }

    private static string NormalizeExistingDirectory(string? value, string displayName)
    {
        var path = NormalizePath(value, displayName);
        if (!Directory.Exists(path))
        {
            throw new ArgumentException($"{displayName}不存在。", displayName);
        }
        AssertNotReparsePoint(path, displayName);
        return path;
    }

    private static string NormalizeInstallRoot(string? value, bool mustExist)
    {
        var path = NormalizePath(value, "安装目录");
        var root = Path.GetPathRoot(path);
        if (string.Equals(path.TrimEnd(Path.DirectorySeparatorChar), root?.TrimEnd(Path.DirectorySeparatorChar), StringComparison.OrdinalIgnoreCase))
        {
            throw new ArgumentException("安装目录不能是磁盘根目录。", nameof(value));
        }

        if (Directory.Exists(path))
        {
            AssertNotReparsePoint(path, "安装目录");
            var unexpectedEntry = Directory.EnumerateFileSystemEntries(path)
                .Select(Path.GetFileName)
                .FirstOrDefault(name => !AllowedInstallRootEntries.Contains(name, StringComparer.OrdinalIgnoreCase));
            if (unexpectedEntry is not null)
            {
                throw new ArgumentException("安装目录包含非安装器管理的内容，请选择专用目录。", nameof(value));
            }
            return path;
        }

        if (mustExist)
        {
            throw new ArgumentException("安装目录不存在。", nameof(value));
        }

        var parent = Directory.GetParent(path)?.FullName;
        if (parent is null || !Directory.Exists(parent))
        {
            throw new ArgumentException("安装目录的父目录不存在。", nameof(value));
        }
        AssertNotReparsePoint(parent, "安装目录父目录");
        return path;
    }

    private static string NormalizePath(string? value, string displayName)
    {
        if (string.IsNullOrWhiteSpace(value) || value.Length > 32_767 || value.Any(char.IsControl))
        {
            throw new ArgumentException($"{displayName}为空或格式无效。", displayName);
        }

        try
        {
            return Path.GetFullPath(value);
        }
        catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
        {
            throw new ArgumentException($"{displayName}格式无效。", displayName);
        }
    }

    private static string? NormalizeVersion(string? value, bool required)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            if (!required)
            {
                return null;
            }
            throw new ArgumentException("版本号不能为空。", nameof(value));
        }

        var version = value.Trim();
        if (!VersionPattern().IsMatch(version))
        {
            throw new ArgumentException("版本号只能包含字母、数字、点、下划线和连字符，且最多 64 个字符。", nameof(value));
        }
        return version;
    }

    private static void AssertNotReparsePoint(string path, string displayName)
    {
        if ((File.GetAttributes(path) & FileAttributes.ReparsePoint) != 0)
        {
            throw new ArgumentException($"{displayName}不能是重解析点。", displayName);
        }
    }

    private static void AssertSeparateRoots(string packageRoot, string installRoot)
    {
        var packagePrefix = packageRoot.TrimEnd(Path.DirectorySeparatorChar) + Path.DirectorySeparatorChar;
        var installPrefix = installRoot.TrimEnd(Path.DirectorySeparatorChar) + Path.DirectorySeparatorChar;
        if (packageRoot.StartsWith(installPrefix, StringComparison.OrdinalIgnoreCase)
            || installRoot.StartsWith(packagePrefix, StringComparison.OrdinalIgnoreCase)
            || string.Equals(packageRoot, installRoot, StringComparison.OrdinalIgnoreCase))
        {
            throw new ArgumentException("发布包目录与安装目录不能相同或互相包含。", nameof(packageRoot));
        }
    }

    private static void AddRequireSigned(List<string> arguments, bool requireSigned)
    {
        if (requireSigned)
        {
            arguments.Add("-RequireSigned");
        }
    }

    private static void AddWhatIf(List<string> arguments, bool whatIf)
    {
        if (whatIf)
        {
            arguments.Add("-WhatIf");
        }
    }

    [GeneratedRegex("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$", RegexOptions.CultureInvariant)]
    private static partial Regex VersionPattern();
}
