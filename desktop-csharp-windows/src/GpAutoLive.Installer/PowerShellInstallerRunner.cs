using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Text;

namespace GpAutoLive.Installer;

internal sealed record InstallerScriptResult(
    int ExitCode,
    string StandardOutput,
    string StandardError,
    bool Cancelled,
    bool TimedOut)
{
    public bool ProcessSucceeded => ExitCode == 0 && !Cancelled && !TimedOut;
}

internal sealed class PowerShellInstallerRunner
{
    private static readonly HashSet<string> AllowedScripts = new(StringComparer.OrdinalIgnoreCase)
    {
        "bootstrap-csharp-windows-runtime.ps1",
        "get-csharp-windows-install-state.ps1",
        "install-csharp-windows-package.ps1",
        "rollback-csharp-windows-package.ps1",
        "uninstall-csharp-windows-package.ps1",
        "verify-release-package.ps1"
    };

    private readonly string _toolsRoot;
    private readonly string _powerShellExecutable;
    private bool _powerShell7Verified;

    public PowerShellInstallerRunner(string toolsRoot)
        : this(toolsRoot, ResolvePowerShellExecutable())
    {
    }

    internal PowerShellInstallerRunner(string toolsRoot, string powerShellExecutable)
    {
        _toolsRoot = Path.GetFullPath(toolsRoot);
        _powerShellExecutable = powerShellExecutable;
    }

    public async Task<InstallerScriptResult> RunAsync(
        InstallerScriptInvocation invocation,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(invocation);
        var scriptPath = ResolveScriptPath(invocation.ScriptName);
        if (!_powerShell7Verified)
        {
            var probe = await RunPowerShellVersionProbeAsync(cancellationToken).ConfigureAwait(false);
            if (!probe.ProcessSucceeded || !probe.StandardOutput.StartsWith("PowerShell 7.", StringComparison.OrdinalIgnoreCase))
            {
                return probe with { ExitCode = -1, StandardError = "powershell_7_required" };
            }
            _powerShell7Verified = true;
        }

        var startInfo = CreateBaseStartInfo();
        startInfo.ArgumentList.Add("-NoLogo");
        startInfo.ArgumentList.Add("-NoProfile");
        startInfo.ArgumentList.Add("-NonInteractive");
        startInfo.ArgumentList.Add("-File");
        startInfo.ArgumentList.Add(scriptPath);
        foreach (var argument in invocation.Arguments)
        {
            startInfo.ArgumentList.Add(argument);
        }

        return await RunWithTimeoutAsync(startInfo, TimeSpan.FromMinutes(10), cancellationToken)
            .ConfigureAwait(false);
    }

    internal string ResolveScriptPath(string scriptName)
    {
        if (!AllowedScripts.Contains(scriptName))
        {
            throw new InvalidOperationException("脚本不在安装维护白名单中。 ");
        }
        if (!Directory.Exists(_toolsRoot)
            || (File.GetAttributes(_toolsRoot) & FileAttributes.ReparsePoint) != 0)
        {
            throw new InvalidOperationException("安装维护脚本目录缺失或不可信。 ");
        }

        var path = Path.GetFullPath(Path.Combine(_toolsRoot, scriptName));
        var prefix = _toolsRoot.TrimEnd(Path.DirectorySeparatorChar) + Path.DirectorySeparatorChar;
        if (!path.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)
            || !File.Exists(path)
            || (File.GetAttributes(path) & FileAttributes.ReparsePoint) != 0)
        {
            throw new InvalidOperationException("安装维护脚本缺失或不可信。 ");
        }
        return path;
    }

    private async Task<InstallerScriptResult> RunPowerShellVersionProbeAsync(CancellationToken cancellationToken)
    {
        var startInfo = CreateBaseStartInfo();
        startInfo.ArgumentList.Add("--version");
        return await RunWithTimeoutAsync(startInfo, TimeSpan.FromSeconds(10), cancellationToken)
            .ConfigureAwait(false);
    }

    private ProcessStartInfo CreateBaseStartInfo() => new()
    {
        FileName = _powerShellExecutable,
        WorkingDirectory = AppContext.BaseDirectory,
        UseShellExecute = false,
        CreateNoWindow = true,
        RedirectStandardOutput = true,
        RedirectStandardError = true
    };

    private static string ResolvePowerShellExecutable()
    {
        var candidates = new List<string>();
        var programFiles = Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles);
        if (!string.IsNullOrWhiteSpace(programFiles))
        {
            candidates.Add(Path.Combine(programFiles, "PowerShell", "7", "pwsh.exe"));
        }

        var pathValue = Environment.GetEnvironmentVariable("PATH");
        if (!string.IsNullOrWhiteSpace(pathValue))
        {
            foreach (var directory in pathValue.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries))
            {
                try
                {
                    candidates.Add(Path.Combine(directory.Trim(), "pwsh.exe"));
                }
                catch (ArgumentException)
                {
                    // Ignore malformed PATH entries and keep probing fixed candidates.
                }
            }
        }

        foreach (var candidate in candidates.Distinct(StringComparer.OrdinalIgnoreCase))
        {
            try
            {
                var fullPath = Path.GetFullPath(candidate);
                if (File.Exists(fullPath)
                    && (File.GetAttributes(fullPath) & FileAttributes.ReparsePoint) == 0)
                {
                    return fullPath;
                }
            }
            catch (Exception exception) when (
                exception is ArgumentException
                or IOException
                or NotSupportedException
                or UnauthorizedAccessException)
            {
                // Continue to the next fixed candidate.
            }
        }

        return "pwsh.exe";
    }

    private static async Task<InstallerScriptResult> RunWithTimeoutAsync(
        ProcessStartInfo startInfo,
        TimeSpan timeout,
        CancellationToken cancellationToken)
    {
        using var timeoutSource = new CancellationTokenSource(timeout);
        using var linkedSource = CancellationTokenSource.CreateLinkedTokenSource(
            cancellationToken,
            timeoutSource.Token);
        try
        {
            return await InstallerProcessRunner.RunAsync(startInfo, linkedSource.Token)
                .ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return new(
                -1,
                string.Empty,
                cancellationToken.IsCancellationRequested ? "cancelled" : "timeout",
                cancellationToken.IsCancellationRequested,
                !cancellationToken.IsCancellationRequested);
        }
        catch (Win32Exception)
        {
            return new(-1, string.Empty, "powershell_7_required", false, false);
        }
        catch (InvalidDataException)
        {
            return new(-1, string.Empty, "output_limit", false, false);
        }
    }
}

internal static class InstallerProcessRunner
{
    internal const int MaxOutputCharacters = 256 * 1024;

    public static async Task<InstallerScriptResult> RunAsync(
        ProcessStartInfo startInfo,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(startInfo);
        using var process = new Process { StartInfo = startInfo };
        if (!process.Start())
        {
            throw new InvalidOperationException("安装维护进程未启动。 ");
        }

        var standardOutput = ReadBoundedAsync(process.StandardOutput, cancellationToken);
        var standardError = ReadBoundedAsync(process.StandardError, cancellationToken);
        try
        {
            await Task.WhenAll(
                    process.WaitForExitAsync(cancellationToken),
                    standardOutput,
                    standardError)
                .ConfigureAwait(false);
        }
        catch
        {
            TryKill(process);
            await WaitForExitAfterKillAsync(process).ConfigureAwait(false);
            await ObserveReaderTasksAsync(standardOutput, standardError).ConfigureAwait(false);
            throw;
        }

        return new(
            process.ExitCode,
            await standardOutput.ConfigureAwait(false),
            await standardError.ConfigureAwait(false),
            false,
            false);
    }

    private static async Task<string> ReadBoundedAsync(
        StreamReader reader,
        CancellationToken cancellationToken)
    {
        var builder = new StringBuilder();
        var buffer = new char[4096];
        while (true)
        {
            var count = await reader.ReadAsync(buffer.AsMemory(), cancellationToken).ConfigureAwait(false);
            if (count == 0)
            {
                return builder.ToString();
            }
            if (builder.Length + count > MaxOutputCharacters)
            {
                throw new InvalidDataException("Installer process output exceeded the bounded limit.");
            }
            builder.Append(buffer, 0, count);
        }
    }

    private static void TryKill(Process process)
    {
        try
        {
            if (!process.HasExited)
            {
                process.Kill(entireProcessTree: true);
            }
        }
        catch (Exception exception) when (exception is InvalidOperationException or Win32Exception)
        {
            // Process exit races are harmless; the bounded wait below still joins it.
        }
    }

    private static async Task WaitForExitAfterKillAsync(Process process)
    {
        try
        {
            await process.WaitForExitAsync(CancellationToken.None)
                .WaitAsync(TimeSpan.FromSeconds(5))
                .ConfigureAwait(false);
        }
        catch (Exception exception) when (exception is InvalidOperationException or TimeoutException)
        {
            // The original cancellation/error remains the actionable result.
        }
    }

    private static async Task ObserveReaderTasksAsync(
        Task<string> standardOutput,
        Task<string> standardError)
    {
        try
        {
            await Task.WhenAll(standardOutput, standardError).ConfigureAwait(false);
        }
        catch (Exception)
        {
            // Preserve the original cancellation/output-limit failure while observing reader faults.
        }
    }
}
