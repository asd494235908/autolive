using System.Buffers;
using System.Diagnostics;
using System.Text;
using GpAutoLive.Core.Processes;

namespace GpAutoLive.Windows;

/// <summary>
/// Windows 原生外部进程适配器。FFprobe 仍只通过 Core 中的进程合同执行；
/// mpv 宿主另行消费 Media 的已验证启动计划，依赖方向不形成循环。
/// </summary>
public sealed class WindowsExternalProcessRunner : IExternalProcessRunner
{
    private const int MaxArgumentCount = 128;
    private const int MaxArgumentCharacters = 32_000;
    private const int MaxCommandCharacters = 32_767;
    private const int ReadBufferSize = 64 * 1024;
    private static readonly TimeSpan CleanupTimeout = TimeSpan.FromSeconds(2);
    private static readonly UTF8Encoding Utf8 = new(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: false);

    /// <summary>启动并有界回收一个受管外部进程。</summary>
    public async Task<ExternalProcessResult> RunAsync(
        ExternalProcessPlan plan,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(plan);

        if (!OperatingSystem.IsWindows() || cancellationToken.IsCancellationRequested)
        {
            return cancellationToken.IsCancellationRequested ? Cancelled() : StartFailed();
        }

        if (!TryValidatePlan(plan))
        {
            return StartFailed();
        }

        using var processJob = WindowsJobObject.TryCreate(out var createdJob)
            ? createdJob
            : null;
        using var process = new Process();

        try
        {
            process.StartInfo = CreateStartInfo(plan);
            process.EnableRaisingEvents = false;
            if (!process.Start())
            {
                return StartFailed();
            }

            if (processJob is not null && !processJob.TryAssign(process))
            {
                // Job Object is an additional containment boundary. If this host or
                // process cannot be assigned, retain the existing bounded Process API.
                processJob.Dispose();
            }
        }
        catch (Exception exception) when (IsProcessStartFailure(exception))
        {
            return StartFailed();
        }

        using var runCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        runCancellation.CancelAfter(plan.Timeout);

        var outputLimitValue = 0;
        void SignalOutputLimit(OutputLimitKind kind)
        {
            if (Interlocked.CompareExchange(ref outputLimitValue, (int)kind, 0) == 0)
            {
                try
                {
                    runCancellation.Cancel();
                }
                catch (ObjectDisposedException)
                {
                    // The process is already being cleaned up; the captured limit remains authoritative.
                }
            }
        }

        var standardOutputTask = ReadBoundedAsync(
            process.StandardOutput.BaseStream,
            plan.MaxStandardOutputBytes,
            runCancellation.Token,
            () => SignalOutputLimit(OutputLimitKind.StandardOutput));
        var standardErrorTask = ReadBoundedAsync(
            process.StandardError.BaseStream,
            plan.MaxStandardErrorBytes,
            runCancellation.Token,
            () => SignalOutputLimit(OutputLimitKind.StandardError));

        var mustTerminate = false;
        var processExitedBeforeCancellation = false;
        try
        {
            await process.WaitForExitAsync(runCancellation.Token).ConfigureAwait(false);
            processExitedBeforeCancellation = true;
        }
        catch (OperationCanceledException) when (runCancellation.IsCancellationRequested)
        {
            mustTerminate = true;
        }

        if (processExitedBeforeCancellation)
        {
            // Do not let a slow pipe drain turn an already exited process into a timeout.
            runCancellation.CancelAfter(Timeout.InfiniteTimeSpan);
        }

        var outputLimit = (OutputLimitKind)Volatile.Read(ref outputLimitValue);
        if (outputLimit != OutputLimitKind.None)
        {
            mustTerminate = true;
        }

        if (mustTerminate)
        {
            KillProcessTree(process, processJob);
            await WaitForExitBoundedAsync(process).ConfigureAwait(false);
        }

        var output = await JoinOutputTasksAsync(
            standardOutputTask,
            standardErrorTask).ConfigureAwait(false);

        if (outputLimit == OutputLimitKind.StandardOutput)
        {
            return new ExternalProcessResult(
                ExternalProcessRunStatus.StandardOutputLimitExceeded,
                SafeExitCode(process),
                output.StandardOutput,
                output.StandardError);
        }

        if (outputLimit == OutputLimitKind.StandardError)
        {
            return new ExternalProcessResult(
                ExternalProcessRunStatus.StandardErrorLimitExceeded,
                SafeExitCode(process),
                output.StandardOutput,
                output.StandardError);
        }

        if (cancellationToken.IsCancellationRequested)
        {
            return new ExternalProcessResult(
                ExternalProcessRunStatus.Cancelled,
                SafeExitCode(process),
                output.StandardOutput,
                output.StandardError);
        }

        if (runCancellation.IsCancellationRequested)
        {
            return new ExternalProcessResult(
                ExternalProcessRunStatus.TimedOut,
                SafeExitCode(process),
                output.StandardOutput,
                output.StandardError);
        }

        if (output.Status == BoundedOutputStatus.Failed)
        {
            return new ExternalProcessResult(
                ExternalProcessRunStatus.OutputReadFailed,
                SafeExitCode(process),
                output.StandardOutput,
                output.StandardError);
        }

        return new ExternalProcessResult(
            ExternalProcessRunStatus.Completed,
            SafeExitCode(process),
            output.StandardOutput,
            output.StandardError);
    }

    private static ProcessStartInfo CreateStartInfo(ExternalProcessPlan plan)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = plan.ExecutablePath,
            UseShellExecute = plan.LaunchPolicy.UseShellExecute,
            CreateNoWindow = plan.LaunchPolicy.CreateNoWindow,
            RedirectStandardOutput = plan.LaunchPolicy.RedirectStandardOutput,
            RedirectStandardError = plan.LaunchPolicy.RedirectStandardError,
        };

        foreach (var argument in plan.Arguments)
        {
            startInfo.ArgumentList.Add(argument);
        }

        return startInfo;
    }

    private static bool TryValidatePlan(ExternalProcessPlan plan)
    {
        try
        {
            if (string.IsNullOrWhiteSpace(plan.ExecutablePath)
                || plan.ExecutablePath.Any(char.IsControl)
                || !Path.IsPathFullyQualified(plan.ExecutablePath))
            {
                return false;
            }

            if (plan.Arguments.IsDefault || plan.Arguments.Length > MaxArgumentCount)
            {
                return false;
            }

            var commandCharacters = plan.ExecutablePath.Length;
            foreach (var argument in plan.Arguments)
            {
                if (argument is null || argument.Length > MaxArgumentCharacters)
                {
                    return false;
                }

                commandCharacters = checked(commandCharacters + argument.Length + 1);
            }

            if (commandCharacters > MaxCommandCharacters)
            {
                return false;
            }

            if (plan.Timeout <= TimeSpan.Zero || plan.Timeout > TimeSpan.FromHours(1))
            {
                return false;
            }

            if (plan.MaxStandardOutputBytes is < 1 or > 16 * 1024 * 1024
                || plan.MaxStandardErrorBytes is < 1 or > 4 * 1024 * 1024)
            {
                return false;
            }

            var policy = plan.LaunchPolicy;
            return policy is not null
                && !policy.UseShellExecute
                && policy.CreateNoWindow
                && policy.RedirectStandardOutput
                && policy.RedirectStandardError
                && policy.KillProcessTreeOnTimeout
                && policy.KillProcessTreeOnCancellation;
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (NotSupportedException)
        {
            return false;
        }
    }

    private static async Task<BoundedOutput> ReadBoundedAsync(
        Stream stream,
        int maximumBytes,
        CancellationToken cancellationToken,
        Action onLimit)
    {
        var rented = ArrayPool<byte>.Shared.Rent(Math.Min(ReadBufferSize, Math.Max(4096, maximumBytes)));
        using var output = new MemoryStream();
        var totalBytes = 0;

        try
        {
            while (true)
            {
                var remainingPlusOne = maximumBytes - totalBytes + 1;
                var requestedBytes = Math.Min(rented.Length, Math.Max(1, remainingPlusOne));
                var bytesRead = await stream.ReadAsync(
                    rented.AsMemory(0, requestedBytes),
                    cancellationToken).ConfigureAwait(false);
                if (bytesRead == 0)
                {
                    return CompleteOutput(output);
                }

                var acceptedBytes = Math.Min(bytesRead, maximumBytes - totalBytes);
                if (acceptedBytes > 0)
                {
                    output.Write(rented, 0, acceptedBytes);
                    totalBytes += acceptedBytes;
                }

                if (bytesRead > acceptedBytes)
                {
                    onLimit();
                    return new BoundedOutput(
                        BoundedOutputStatus.LimitExceeded,
                        Decode(output));
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return new BoundedOutput(BoundedOutputStatus.Cancelled, Decode(output));
        }
        catch (IOException)
        {
            return new BoundedOutput(BoundedOutputStatus.Failed, Decode(output));
        }
        catch (ObjectDisposedException)
        {
            return new BoundedOutput(BoundedOutputStatus.Failed, Decode(output));
        }
        finally
        {
            ArrayPool<byte>.Shared.Return(rented);
        }
    }

    private static BoundedOutput CompleteOutput(MemoryStream output) =>
        new(BoundedOutputStatus.Completed, Decode(output));

    private static string Decode(MemoryStream output) =>
        Utf8.GetString(output.GetBuffer(), 0, checked((int)output.Length));

    private static async Task<JoinedOutput> JoinOutputTasksAsync(
        Task<BoundedOutput> standardOutputTask,
        Task<BoundedOutput> standardErrorTask)
    {
        try
        {
            var joined = await Task.WhenAll(standardOutputTask, standardErrorTask)
                .WaitAsync(CleanupTimeout)
                .ConfigureAwait(false);
            return new JoinedOutput(
                joined[0].Status == BoundedOutputStatus.Failed
                    || joined[1].Status == BoundedOutputStatus.Failed
                    ? BoundedOutputStatus.Failed
                    : joined[0].Status,
                joined[0].Text,
                joined[1].Text);
        }
        catch (TimeoutException)
        {
            // Kill(entireProcessTree:true) should close inherited pipes. If an exotic child
            // keeps a pipe open, return a bounded result and let Process.Dispose close handles.
            return new JoinedOutput(BoundedOutputStatus.Failed, string.Empty, string.Empty);
        }
        catch (OperationCanceledException)
        {
            return new JoinedOutput(BoundedOutputStatus.Failed, string.Empty, string.Empty);
        }
    }

    private static async Task WaitForExitBoundedAsync(Process process)
    {
        try
        {
            await process.WaitForExitAsync().WaitAsync(CleanupTimeout).ConfigureAwait(false);
        }
        catch (InvalidOperationException)
        {
            // The process exited between the cancellation callback and cleanup.
        }
        catch (TimeoutException)
        {
            // The handle is disposed by the owner after this bounded cleanup window.
        }
    }

    private static void KillProcessTree(Process process, WindowsJobObject? processJob)
    {
        if (processJob is not null && processJob.TryTerminate())
        {
            return;
        }

        try
        {
            if (!process.HasExited)
            {
                process.Kill(entireProcessTree: true);
            }
        }
        catch (InvalidOperationException)
        {
            // Process already exited or could not expose a live handle.
        }
        catch (System.ComponentModel.Win32Exception)
        {
            // The OS denied/removed the handle; cleanup remains bounded and the result is not success.
        }
        catch (PlatformNotSupportedException)
        {
            // The project is Windows-only; keep the boundary fail-closed if called elsewhere.
        }
    }

    private static int? SafeExitCode(Process process)
    {
        try
        {
            return process.HasExited ? process.ExitCode : null;
        }
        catch (InvalidOperationException)
        {
            return null;
        }
        catch (System.ComponentModel.Win32Exception)
        {
            return null;
        }
    }

    private static bool IsProcessStartFailure(Exception exception) =>
        exception is InvalidOperationException
            or System.ComponentModel.Win32Exception
            or ArgumentException
            or NotSupportedException
            or UnauthorizedAccessException
            or System.Security.SecurityException;

    private static ExternalProcessResult StartFailed() =>
        new(ExternalProcessRunStatus.StartFailed, null, string.Empty, string.Empty);

    private static ExternalProcessResult Cancelled() =>
        new(ExternalProcessRunStatus.Cancelled, null, string.Empty, string.Empty);

    private enum OutputLimitKind
    {
        None,
        StandardOutput,
        StandardError,
    }

    private enum BoundedOutputStatus
    {
        Completed,
        LimitExceeded,
        Cancelled,
        Failed,
    }

    private readonly record struct BoundedOutput(BoundedOutputStatus Status, string Text);

    private readonly record struct JoinedOutput(
        BoundedOutputStatus Status,
        string StandardOutput,
        string StandardError);
}
