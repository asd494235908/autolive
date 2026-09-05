using System.Collections.Immutable;
using System.Diagnostics;
using GpAutoLive.Core.Processes;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class ExternalProcessBoundaryTests
{
    [TestMethod]
    public async Task Runner_captures_stdout_and_stderr_without_shell_interpolation()
    {
        var result = await RunCommandAsync(
            ["/c", "echo standard-output & echo standard-error 1>&2"],
            TimeSpan.FromSeconds(5));

        Assert.AreEqual(ExternalProcessRunStatus.Completed, result.Status);
        Assert.AreEqual(0, result.ExitCode);
        StringAssert.Contains(result.StandardOutput, "standard-output");
        StringAssert.Contains(result.StandardError, "standard-error");
    }

    [TestMethod]
    public async Task Runner_rejects_a_shell_enabled_plan_before_starting_any_process()
    {
        var policy = ProcessLaunchPolicy.HiddenNoShellProcessTree with { UseShellExecute = true };
        var result = await RunCommandAsync(
            ["/c", "echo should-not-run"],
            TimeSpan.FromSeconds(5),
            policy);

        Assert.AreEqual(ExternalProcessRunStatus.StartFailed, result.Status);
        Assert.IsNull(result.ExitCode);
        Assert.AreEqual(string.Empty, result.StandardOutput);
        Assert.AreEqual(string.Empty, result.StandardError);
    }

    [TestMethod]
    public async Task Runner_times_out_and_kills_the_process_tree()
    {
        var stopwatch = Stopwatch.StartNew();
        var result = await RunCommandAsync(
            ["/c", "ping.exe 127.0.0.1 -n 30 -w 1000 >nul"],
            TimeSpan.FromMilliseconds(300));
        stopwatch.Stop();

        Assert.AreEqual(ExternalProcessRunStatus.TimedOut, result.Status);
        Assert.IsTrue(stopwatch.Elapsed < TimeSpan.FromSeconds(5));
    }

    [TestMethod]
    public async Task Runner_honors_cancellation_and_kills_the_process_tree()
    {
        using var cancellation = new CancellationTokenSource(TimeSpan.FromMilliseconds(300));
        var result = await RunCommandAsync(
            ["/c", "ping.exe 127.0.0.1 -n 30 -w 1000 >nul"],
            TimeSpan.FromSeconds(30),
            cancellationToken: cancellation.Token);

        Assert.AreEqual(ExternalProcessRunStatus.Cancelled, result.Status);
    }

    [TestMethod]
    public async Task Runner_stops_when_stdout_reaches_the_configured_byte_limit()
    {
        var result = await RunCommandAsync(
            ["/c", "for /L %i in (1,1,100000) do @echo 0123456789"],
            TimeSpan.FromSeconds(5),
            maxStandardOutputBytes: 64);

        Assert.AreEqual(ExternalProcessRunStatus.StandardOutputLimitExceeded, result.Status);
        Assert.IsTrue(result.StandardOutput.Length <= 64);
    }

    [TestMethod]
    public async Task Runner_stops_when_stderr_reaches_the_configured_byte_limit()
    {
        var result = await RunCommandAsync(
            ["/c", "for /L %i in (1,1,100000) do @echo 0123456789 1>&2"],
            TimeSpan.FromSeconds(5),
            maxStandardErrorBytes: 64);

        Assert.AreEqual(ExternalProcessRunStatus.StandardErrorLimitExceeded, result.Status);
        Assert.IsTrue(result.StandardError.Length <= 64);
    }

    [TestMethod]
    public async Task Runner_returns_a_sanitized_start_failure_for_a_missing_executable()
    {
        var plan = new ExternalProcessPlan(
            Path.Combine(Path.GetTempPath(), $"missing-{Guid.NewGuid():N}.exe"),
            ["/c", "echo should-not-run"],
            ProcessLaunchPolicy.HiddenNoShellProcessTree,
            TimeSpan.FromSeconds(5),
            4096,
            4096);

        var result = await new WindowsExternalProcessRunner().RunAsync(plan, CancellationToken.None);

        Assert.AreEqual(ExternalProcessRunStatus.StartFailed, result.Status);
        Assert.IsNull(result.ExitCode);
        Assert.AreEqual(string.Empty, result.StandardOutput);
        Assert.AreEqual(string.Empty, result.StandardError);
    }

    [TestMethod]
    public void Job_object_assigns_and_terminates_a_process_or_fails_closed_when_unavailable()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        var created = WindowsJobObject.TryCreate(out var job);
        if (!created)
        {
            Assert.IsNull(job);
            return;
        }

        Assert.IsNotNull(job);
        var processJob = job!;
        using (processJob)
        {
            using var process = StartLongLivedProcess();
            if (!processJob.TryAssign(process))
            {
                // Nested-job policy or host restrictions are an allowed unavailable path;
                // the caller must keep its existing bounded Process cleanup fallback.
                Assert.IsFalse(process.HasExited);
                process.Kill(entireProcessTree: true);
                process.WaitForExit(2000);
                return;
            }

            Assert.IsTrue(processJob.TryTerminate(1234));
            Assert.IsTrue(process.WaitForExit(2000));
        }
    }

    [TestMethod]
    public void Disposed_job_object_rejects_new_operations()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        var created = WindowsJobObject.TryCreate(out var job);
        if (!created)
        {
            Assert.IsNull(job);
            return;
        }

        Assert.IsNotNull(job);
        job.Dispose();
        Assert.IsFalse(job.TryTerminate());
    }

    private static async Task<ExternalProcessResult> RunCommandAsync(
        ImmutableArray<string> arguments,
        TimeSpan timeout,
        ProcessLaunchPolicy? policy = null,
        int maxStandardOutputBytes = 4096,
        int maxStandardErrorBytes = 4096,
        CancellationToken cancellationToken = default)
    {
        var command = Environment.GetEnvironmentVariable("ComSpec")
            ?? Path.Combine(Environment.SystemDirectory, "cmd.exe");
        var plan = new ExternalProcessPlan(
            command,
            arguments,
            policy ?? ProcessLaunchPolicy.HiddenNoShellProcessTree,
            timeout,
            maxStandardOutputBytes,
            maxStandardErrorBytes);
        return await new WindowsExternalProcessRunner().RunAsync(plan, cancellationToken);
    }

    private static Process StartLongLivedProcess()
    {
        var command = Environment.GetEnvironmentVariable("ComSpec")
            ?? Path.Combine(Environment.SystemDirectory, "cmd.exe");
        var startInfo = new ProcessStartInfo
        {
            FileName = command,
            UseShellExecute = false,
            CreateNoWindow = true,
        };
        startInfo.ArgumentList.Add("/c");
        startInfo.ArgumentList.Add("ping.exe 127.0.0.1 -n 30 -w 1000 >nul");
        return Process.Start(startInfo)
            ?? throw new InvalidOperationException("无法启动 Job Object 测试进程。");
    }
}
