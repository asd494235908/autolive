using System.Diagnostics;
using System.Reflection;
using GpAutoLive.Contracts;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsRtmpGracefulStopTests
{
    [TestMethod]
    public async Task Stop_success_requires_independent_process_handle_to_be_signalled()
    {
        for (var attempt = 0; attempt < 12; attempt++)
        {
            await using var manager = new WindowsRtmpOutputManager();
            using var process = StartFixture("ping -n 30 127.0.0.1 >nul");
            using var observer = Process.GetProcessById(process.Id);
            _ = observer.Handle; // 在退出前持有独立内核观察句柄。
            Attach(manager, process, true);
            process.Kill(entireProcessTree: true);
            var stopped = await manager.StopAsync();
            Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
            Assert.IsTrue(observer.WaitForExit(0), $"第 {attempt + 1} 次 Stop 返回后内核进程尚未结束。");
        }
    }

    [TestMethod]
    [DataRow(false)]
    [DataRow(true)]
    public async Task Stop_requests_q_or_pcm_eof_before_kill(bool audio)
    {
        await using var manager = new WindowsRtmpOutputManager();
        using var process = StartFixture(audio
            ? "findstr . >nul & if errorlevel 1 (exit /b 0) else (exit /b 9)"
            : "set /p quit= & if \"!quit!\"==\"q\" (exit /b 0) else (exit /b 9)");
        Attach(manager, process, audio);
        Assert.AreEqual(audio, manager.Snapshot.FinalPcmInputOpen);
        var stopped = await manager.StopAsync();
        Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
        Assert.AreEqual(0, stopped.Snapshot.ExitCode, "正常 stdin 结束应当让夹具自主退出。");
    }

    [TestMethod]
    [DataRow(false)]
    [DataRow(true)]
    public async Task Queued_pcm_rechecks_state_and_input_after_entering_write_lock(bool replaceInput)
    {
        await using var manager = new WindowsRtmpOutputManager();
        using var input = new MemoryStream();
        Set(manager, "_standardInput", input);
        Set(manager, "_state", RtmpOutputState.Publishing);
        var serial = Get<SemaphoreSlim>(manager, "_audioSerial");
        await serial.WaitAsync();
        var write = manager.WriteFinalPcmAsync(new float[] { 1, 1 });
        using var replacement = new MemoryStream();
        if (replaceInput) Set(manager, "_standardInput", replacement);
        else Set(manager, "_state", RtmpOutputState.Stopping);
        serial.Release();
        var result = await write;
        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(0L, input.Length);
        Assert.AreEqual(replaceInput ? RtmpOutputState.Publishing : RtmpOutputState.Stopping, manager.Snapshot.State);
    }

    [TestMethod]
    public async Task Stop_kills_uncooperative_child_and_retains_owners_when_writer_lock_times_out()
    {
        await using var manager = new WindowsRtmpOutputManager();
        using var process = StartFixture("ping -n 30 127.0.0.1 >nul");
        Attach(manager, process, true);
        var serial = Get<SemaphoreSlim>(manager, "_audioSerial");
        await serial.WaitAsync();
        WindowsRtmpResult stopped;
        var watch = Stopwatch.StartNew();
        try
        {
            stopped = await manager.StopAsync();
            Assert.IsFalse(stopped.IsSuccess);
            Assert.IsTrue(process.HasExited);
            Assert.IsNotNull(manager.Snapshot.ProcessId);
            Assert.IsTrue(manager.Snapshot.FinalPcmInputOpen);
            Assert.IsTrue(watch.Elapsed < TimeSpan.FromSeconds(2.8), $"清理超过共享期限：{watch.Elapsed}");
        }
        finally { serial.Release(); }
        Assert.IsTrue((await manager.StopAsync()).IsSuccess);
    }

    [TestMethod]
    public async Task Exited_cleanup_retains_owner_until_stderr_join_completes()
    {
        await using var manager = new WindowsRtmpOutputManager();
        using var process = StartFixture("exit /b 0");
        await process.WaitForExitAsync();
        Set(manager, "_process", process);
        var pending = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        Set(manager, "_stderrTask", pending.Task);
        var release = typeof(WindowsRtmpOutputManager).GetMethod("ReleaseExitedProcess", BindingFlags.Instance | BindingFlags.NonPublic)!;
        try
        {
            Assert.AreEqual(false, release.Invoke(manager, [false]));
            Assert.IsNotNull(manager.Snapshot.ProcessId);
        }
        finally { pending.SetResult(); }
        Assert.IsTrue((await manager.StopAsync()).IsSuccess);
    }

    private static Process StartFixture(string command)
    {
        var info = new ProcessStartInfo(Path.Combine(Environment.SystemDirectory, "cmd.exe"))
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardInput = true,
            RedirectStandardError = true,
        };
        foreach (var argument in new[] { "/d", "/q", "/v:on", "/c", command }) info.ArgumentList.Add(argument);
        return Process.Start(info)!;
    }

    private static void Attach(WindowsRtmpOutputManager manager, Process process, bool audio)
    {
        Set(manager, "_process", process);
        Set(manager, "_state", RtmpOutputState.Publishing);
        Set(manager, "_standardInput", audio ? process.StandardInput.BaseStream : null);
        Set(manager, "_stderrTask", process.StandardError.ReadToEndAsync());
    }

    private static void Set(WindowsRtmpOutputManager manager, string name, object? value) =>
        typeof(WindowsRtmpOutputManager).GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)!.SetValue(manager, value);

    private static T Get<T>(WindowsRtmpOutputManager manager, string name) =>
        (T)typeof(WindowsRtmpOutputManager).GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(manager)!;
}
