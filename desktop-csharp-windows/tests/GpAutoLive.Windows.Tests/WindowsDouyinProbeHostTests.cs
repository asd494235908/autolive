using System.Diagnostics;
using System.Text;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsDouyinProbeHostTests
{
    [TestMethod]
    public async Task Login_creates_a_safe_local_diagnostic_log()
    {
        using var fixture = ProbeFixture.Create();
        await using var host = fixture.CreateHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root));
        Assert.AreEqual(WindowsDouyinDiagnosticLogState.NotStarted, host.Snapshot.DiagnosticLogState);
        Assert.IsNull(host.Snapshot.DiagnosticLogPath);
        Assert.IsTrue((await host.StartAsync(fixture.Request with { Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson })).IsSuccess);
        Assert.AreEqual(WindowsDouyinDiagnosticLogState.Ready, host.Snapshot.DiagnosticLogState);
        Assert.IsTrue(File.Exists(host.Snapshot.DiagnosticLogPath));
    }

    [TestMethod]
    public async Task Rejected_diagnostics_and_raw_stderr_are_never_saved_or_applied_to_chat_state()
    {
        using var fixture = ProbeFixture.Create();
        await using var host = fixture.CreateHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, emitDiagnostics: true));
        var listening = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening) listening.TrySetResult();
        };
        Assert.IsTrue((await host.StartAsync(fixture.Request with { Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson })).IsSuccess);
        await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.AreEqual(0, host.Snapshot.InvalidEventCount);
        Assert.IsTrue(host.Snapshot.Authenticated);
        Assert.IsTrue((await host.StopAsync()).IsSuccess);
        var content = File.ReadAllText(host.Snapshot.DiagnosticLogPath!);
        Assert.IsTrue(content.Contains("diagnostic_rejected", StringComparison.Ordinal));
        Assert.IsTrue(content.Contains("verification_required", StringComparison.Ordinal));
        Assert.IsTrue(content.Contains("ExplicitStop", StringComparison.Ordinal));
        Assert.IsFalse(content.Contains("do-not-save-secret", StringComparison.Ordinal));
        Assert.IsFalse(content.Contains("png_base64", StringComparison.Ordinal));
    }

    [TestMethod]
    public async Task Diagnostic_file_failure_does_not_prevent_login_and_is_projected_to_snapshot()
    {
        using var fixture = ProbeFixture.Create();
        var blocked = Path.Combine(fixture.Root, "blocked-log");
        File.WriteAllText(blocked, "keep");
        await using var host = new WindowsDouyinProbeHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root), diagnosticLog: new WindowsDouyinDiagnosticLog(blocked));
        var listening = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening) listening.TrySetResult();
        };
        Assert.IsTrue((await host.StartAsync(fixture.Request with { Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson })).IsSuccess);
        await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.IsTrue(host.Snapshot.Authenticated);
        Assert.AreEqual(WindowsDouyinDiagnosticLogState.WriteFailed, host.Snapshot.DiagnosticLogState);
        Assert.IsNull(host.Snapshot.DiagnosticLogPath);
    }
    [TestMethod]
    public async Task Manual_send_requires_connected_session_and_valid_content()
    {
        await using var host = new WindowsDouyinProbeHost(new DouyinLiveManager());
        Assert.AreEqual(DouyinSendOutcome.NotSent, (await host.SendChatAsync("hello")).Outcome);
        Assert.AreEqual(DouyinSendOutcome.NotSent, (await host.SendChatAsync(null)).Outcome);
    }

    [TestMethod]
    public async Task Manual_send_in_watch_mode_is_single_flight_with_unicode_validation_and_real_inbound_echo()
    {
        using var fixture = ProbeFixture.Create();
        var marker = Path.Combine(fixture.Root, "manual.log");
        var manager = new DouyinLiveManager();
        await using var host = fixture.CreateHost(manager,
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, gracefulMarkerPath: marker));
        var listening = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var echo = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening) listening.TrySetResult();
            if (host.GetChatMessages().Any(message => message.IsSelf)) echo.TrySetResult();
        };
        Assert.IsTrue((await host.StartAsync(fixture.Request with
        {
            Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson,
            Config = fixture.Request.Config with { Enabled = false, Replies = [] }
        })).IsSuccess);
        await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.IsTrue(manager.Pause().IsSuccess);
        Assert.AreEqual(DouyinSendOutcome.NotSent, (await host.SendChatAsync("paused")).Outcome);
        Assert.IsTrue(manager.Resume().IsSuccess);
        Assert.AreEqual(DouyinSendOutcome.NotSent, (await host.SendChatAsync(new string('a', 81))).Outcome);
        Assert.AreEqual(DouyinSendOutcome.NotSent, (await host.SendChatAsync("a\nb")).Outcome);
        var content = string.Concat(Enumerable.Repeat("😀", 80));
        var results = await Task.WhenAll(host.SendChatAsync(content), host.SendChatAsync(content));
        Assert.AreEqual(1, results.Count(result => result.Outcome == DouyinSendOutcome.Accepted));
        Assert.AreEqual(1, results.Count(result => result.Outcome == DouyinSendOutcome.NotSent));
        await echo.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.AreEqual(content, host.GetChatMessages().Single().Text);
        Assert.AreEqual(0UL, host.Snapshot.Douyin.Metrics.Enqueued);
        Assert.AreEqual(1UL, host.Snapshot.Douyin.Metrics.Accepted);
        Assert.AreEqual(DouyinSendOutcome.NotSent, (await host.SendChatAsync("rate limited")).Outcome);
        Assert.IsTrue((await host.DisconnectAsync()).IsSuccess);
        Assert.AreEqual(DouyinSendOutcome.NotSent, (await host.SendChatAsync("old room")).Outcome);
        Assert.AreEqual(1, File.ReadAllLines(marker).Count(value => value == "chat.send"));
    }

    [TestMethod]
    [DataRow("unknown")]
    [DataRow("hold")]
    public async Task Manual_send_unknown_or_cancel_after_dispatch_is_not_retried(string mode)
    {
        using var fixture = ProbeFixture.Create();
        var marker = Path.Combine(fixture.Root, "manual-unknown.log");
        await using var host = fixture.CreateHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, gracefulMarkerPath: marker, manualSendMode: mode));
        var listening = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var dispatched = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening) listening.TrySetResult();
            if (snapshot.Douyin.Metrics.GapEvents > 0) dispatched.TrySetResult();
        };
        Assert.IsTrue((await host.StartAsync(fixture.Request with
        {
            Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson,
            Config = fixture.Request.Config with { Enabled = false, Replies = [] }
        })).IsSuccess);
        await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));
        using var cancellation = new CancellationTokenSource();
        var pending = host.SendChatAsync("manual", cancellation.Token);
        if (mode == "hold")
        {
            await dispatched.Task.WaitAsync(TimeSpan.FromSeconds(5));
            Assert.AreEqual(DouyinSendOutcome.NotSent, (await host.SendChatAsync("busy")).Outcome);
            cancellation.Cancel();
        }
        var result = await pending;
        Assert.AreEqual(DouyinSendOutcome.OutcomeUnknown, result.Outcome);
        Assert.IsTrue(host.GetChatMessages().IsEmpty, "没有平台入站回显时不能创建本人弹幕行");
        Assert.IsTrue((await host.StopAsync()).IsSuccess);
        Assert.AreEqual(1, File.ReadAllLines(marker).Count(value => value == "chat.send"));
    }

    [TestMethod]
    public async Task Disconnect_does_not_wait_for_an_automatic_reply_rate_budget()
    {
        using var fixture = ProbeFixture.Create();
        await using var host = fixture.CreateHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, emitChat: true, emitSecondChat: true));
        var queued = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.Metrics.Enqueued == 2) queued.TrySetResult();
        };
        Assert.IsTrue((await host.StartAsync(fixture.Request with
        {
            Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson
        })).IsSuccess);
        await queued.Task.WaitAsync(TimeSpan.FromSeconds(5));
        var disconnected = await host.DisconnectAsync().WaitAsync(TimeSpan.FromMilliseconds(1500));
        Assert.IsTrue(disconnected.IsSuccess);
        Assert.IsTrue(disconnected.Snapshot.Authenticated);
        Assert.AreEqual(0, disconnected.Snapshot.Douyin.QueueCount);
    }

    private static readonly byte[] OnePixelPng =
    [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
        0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41,
        0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
        0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99,
        0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
        0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82
    ];

    [TestMethod]
    public void Canonical_qr_png_writer_requires_future_valid_png_and_keeps_existing_file()
    {
        using var fixture = ProbeFixture.Create();
        var path = Path.Combine(fixture.Root, "canonical-qr.png");

        Assert.IsTrue(WindowsDouyinProbeHost.TryWriteQrPng(
            path,
            OnePixelPng,
            DateTimeOffset.UtcNow.AddMinutes(5)));
        CollectionAssert.AreEqual(OnePixelPng, File.ReadAllBytes(path));

        Assert.IsFalse(WindowsDouyinProbeHost.TryWriteQrPng(
            path,
            OnePixelPng,
            DateTimeOffset.UtcNow.AddMinutes(5)));
        Assert.IsFalse(WindowsDouyinProbeHost.TryWriteQrPng(
            Path.Combine(fixture.Root, "expired.png"),
            OnePixelPng,
            DateTimeOffset.UtcNow.AddSeconds(-1)));
        Assert.IsFalse(WindowsDouyinProbeHost.TryWriteQrPng(
            Path.Combine(fixture.Root, "not-png.png"),
            [0x01, 0x02],
            DateTimeOffset.UtcNow.AddMinutes(5)));
    }

    [TestMethod]
    public async Task Invalid_plan_fails_before_core_session_starts()
    {
        await using var host = new WindowsDouyinProbeHost(new DouyinLiveManager());

        var result = await host.StartAsync(null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsDouyinProbeHostFailureCode.InvalidPlan, result.Error!.Code);
        Assert.AreEqual(DouyinLiveState.Idle, result.Snapshot.Douyin.State);
        Assert.IsNull(result.Snapshot.ProcessId);
    }

    [TestMethod]
    public async Task Immediately_exiting_probe_is_not_reported_as_success()
    {
        using var fixture = ProbeFixture.Create();
        var manager = new DouyinLiveManager();
        await using var host = fixture.CreateHost(manager);
        var completed = new TaskCompletionSource<WindowsDouyinProbeHostSnapshot>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.State is WindowsDouyinProbeHostState.Exited or WindowsDouyinProbeHostState.Failed)
            {
                completed.TrySetResult(snapshot);
            }
        };

        var start = await host.StartAsync(fixture.Request);
        var terminal = start.Snapshot.State is WindowsDouyinProbeHostState.Exited or WindowsDouyinProbeHostState.Failed
            ? start.Snapshot
            : await completed.Task.WaitAsync(TimeSpan.FromSeconds(3));

        Assert.AreNotEqual(DouyinLiveState.Passed, terminal.Douyin.State);
        Assert.IsFalse(terminal.Douyin.Running);
        var stopped = await host.StopAsync();
        Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
        Assert.AreEqual(WindowsDouyinProbeHostState.Stopped, stopped.Snapshot.State);
        Assert.AreEqual(DouyinLiveState.Idle, stopped.Snapshot.Douyin.State);
    }

    [TestMethod]
    public async Task Canonical_sidecar_bootstraps_qr_login_and_binds_live_open_before_listening()
    {
        using var fixture = ProbeFixture.Create();
        var gracefulMarker = Path.Combine(fixture.Root, "graceful-stop.log");
        var manager = new DouyinLiveManager();
        await using var host = fixture.CreateHost(
            manager,
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, gracefulMarkerPath: gracefulMarker));
        var listening = new TaskCompletionSource<WindowsDouyinProbeHostSnapshot>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening)
            {
                listening.TrySetResult(snapshot);
            }
        };

        var start = await host.StartAsync(fixture.Request with
        {
            Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson
        });
        var snapshot = await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));

        Assert.IsTrue(start.IsSuccess, start.Error?.Message);
        Assert.AreEqual(WindowsDouyinProbeHostState.Running, snapshot.State);
        Assert.AreEqual(DouyinLiveState.Listening, snapshot.Douyin.State);
        Assert.IsTrue(snapshot.Douyin.RoomResolved);
        Assert.IsTrue(snapshot.QrPath is not null && File.Exists(snapshot.QrPath));
        Assert.IsTrue(snapshot.Douyin.Generation > 1);

        var stopped = await host.StopAsync();
        Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
        Assert.AreEqual(WindowsDouyinProbeHostState.Stopped, stopped.Snapshot.State);
        CollectionAssert.AreEqual(
            new[] { "live.close", "auth.logout", "shutdown" },
            File.ReadAllLines(gracefulMarker));
    }

    [TestMethod]
    public async Task Canonical_sidecar_chat_event_is_queued_sent_and_counted_as_accepted()
    {
        using var fixture = ProbeFixture.Create();
        var manager = new DouyinLiveManager();
        await using var host = fixture.CreateHost(
            manager,
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, emitChat: true, emitGap: true));
        var accepted = new TaskCompletionSource<WindowsDouyinProbeHostSnapshot>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.Metrics.Accepted > 0)
            {
                accepted.TrySetResult(snapshot);
            }
        };

        var start = await host.StartAsync(fixture.Request with
        {
            Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson
        });
        WindowsDouyinProbeHostSnapshot snapshot;
        try
        {
            snapshot = await accepted.Task.WaitAsync(TimeSpan.FromSeconds(5));
        }
        catch (TimeoutException)
        {
            var current = host.Snapshot;
            Assert.Fail($"未观察到 accepted；host={current.State} douyin={current.Douyin.State} event={current.LastEvent} invalid={current.InvalidEventCount} metrics={current.Douyin.Metrics}");
            throw;
        }

        Assert.IsTrue(start.IsSuccess, start.Error?.Message);
        Assert.AreEqual(DouyinLiveState.Listening, snapshot.Douyin.State);
        Assert.AreEqual(1UL, snapshot.Douyin.Metrics.Enqueued);
        Assert.AreEqual(1UL, snapshot.Douyin.Metrics.Dequeued);
        Assert.AreEqual(1UL, snapshot.Douyin.Metrics.Accepted);
        Assert.AreEqual(1UL, snapshot.Douyin.Metrics.GapEvents);
        Assert.AreEqual(2UL, snapshot.Douyin.Metrics.GapDroppedCount);
        Assert.AreEqual("reconnect", snapshot.Douyin.LastGapReason);
        Assert.IsTrue(snapshot.Douyin.ReplyAttempted);

        var stopped = await host.StopAsync();
        Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
    }

    [TestMethod]
    public async Task Canonical_watching_continues_over_old_stdout_and_deadline_limits_without_sending()
    {
        using var fixture = ProbeFixture.Create();
        var time = new StartupTimeProvider();
        var manager = new DouyinLiveManager();
        await using var host = fixture.CreateHost(manager,
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, emitBulkChat: true), time);
        var received = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, _) =>
        {
            if (host.GetChatMessages().LastOrDefault()?.MessageId == "bulk-599")
            {
                received.TrySetResult();
            }
        };
        Assert.IsTrue((await host.StartAsync(fixture.Request with
        {
            Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson,
            Config = fixture.Request.Config with { Enabled = false, Replies = [] }
        })).IsSuccess);
        await received.Task.WaitAsync(TimeSpan.FromSeconds(10));
        time.AdvanceBeyondStartupDeadline();
        Assert.IsTrue(time.StartupTimerDisposed);
        Assert.AreEqual(DouyinLiveState.Listening, host.Snapshot.Douyin.State);
        Assert.AreEqual(500, host.GetChatMessages().Length);
        Assert.AreEqual("bulk-100", host.GetChatMessages()[0].MessageId);
        Assert.AreEqual("观众", host.GetChatMessages()[0].Nickname);
        Assert.AreEqual(0UL, host.Snapshot.Douyin.Metrics.Enqueued);
        Assert.IsFalse(host.Snapshot.Douyin.ReplyAttempted);
        Assert.IsTrue((await host.StopAsync()).IsSuccess);
        Assert.AreEqual(500, host.GetChatMessages().Length);
        host.ClearChatMessages();
        Assert.IsTrue(host.GetChatMessages().IsEmpty);
    }

    [TestMethod]
    public async Task Disconnect_and_reopen_keep_one_authenticated_process_and_filter_retired_room_events()
    {
        using var fixture = ProbeFixture.Create();
        var marker = Path.Combine(fixture.Root, "commands.log");
        var manager = new DouyinLiveManager();
        var processStarts = 0;
        await using var host = fixture.CreateHost(manager, _ =>
        {
            processStarts++;
            return CreateCanonicalSidecarStartInfo(fixture.Root, gracefulMarkerPath: marker);
        });
        var listening = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var fresh = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening) listening.TrySetResult();
            if (host.GetChatMessages().LastOrDefault()?.MessageId == "fresh") fresh.TrySetResult();
        };
        var request = fixture.Request with
        {
            Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson,
            Config = fixture.Request.Config with { Enabled = false, Replies = [] }
        };
        Assert.IsTrue((await host.StartAsync(request)).IsSuccess);
        await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));
        var first = host.Snapshot;
        Assert.IsTrue(first.Authenticated);
        Assert.AreEqual(WindowsDouyinLoginClearReason.None, first.LoginClearReason);
        Assert.IsTrue((await host.StartAsync(request)).IsSuccess);
        Assert.AreEqual(first.Douyin.Generation, host.Snapshot.Douyin.Generation);
        Assert.AreEqual(first.ProcessId, host.Snapshot.ProcessId);
        Assert.IsFalse((await host.StartAsync(request with { Config = request.Config with { RoomId = "23456" } })).IsSuccess);
        Assert.IsTrue(manager.BlockReplySending("风险状态").IsSuccess);
        Assert.IsTrue((await host.DisconnectAsync()).IsSuccess);
        Assert.AreEqual(WindowsDouyinProbeHostState.Ready, host.Snapshot.State);
        Assert.AreEqual(DouyinLiveState.LoggedIn, host.Snapshot.Douyin.State);
        Assert.IsTrue(host.Snapshot.Authenticated);
        CollectionAssert.AreEqual(new[] { "live.close" }, File.ReadAllLines(marker));

        Assert.IsTrue((await host.StartAsync(request with { Config = request.Config with { RoomId = "23456" } })).IsSuccess);
        await fresh.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.AreEqual(1, processStarts);
        Assert.AreEqual(first.ProcessId, host.Snapshot.ProcessId);
        Assert.AreNotEqual(first.Douyin.Generation, host.Snapshot.Douyin.Generation);
        Assert.IsTrue(host.Snapshot.Authenticated);
        Assert.IsTrue(host.Snapshot.Douyin.ReplySendingBlocked);
        Assert.AreEqual(1, host.GetChatMessages().Length);
        Assert.AreEqual(0, host.Snapshot.InvalidEventCount);
        Assert.IsTrue((await host.StopAsync()).IsSuccess);
        Assert.IsFalse(host.Snapshot.Authenticated);
        Assert.AreEqual(WindowsDouyinLoginClearReason.ExplicitStop, host.Snapshot.LoginClearReason);
        CollectionAssert.AreEqual(new[] { "live.close", "live.close", "auth.logout", "shutdown" }, File.ReadAllLines(marker));
    }

    [TestMethod]
    public async Task Natural_room_close_keeps_login_and_dispose_destroys_the_process()
    {
        using var fixture = ProbeFixture.Create();
        var host = fixture.CreateHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, naturalClose: true));
        var disconnected = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.State == WindowsDouyinProbeHostState.Ready && snapshot.Authenticated)
                disconnected.TrySetResult();
        };
        var request = fixture.Request with
        {
            Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson,
            Config = fixture.Request.Config with { Enabled = false, Replies = [] }
        };
        try
        {
            Assert.IsTrue((await host.StartAsync(request)).IsSuccess);
            await disconnected.Task.WaitAsync(TimeSpan.FromSeconds(5));
            Assert.AreEqual(DouyinLiveState.LoggedIn, host.Snapshot.Douyin.State);
            Assert.IsTrue(host.Snapshot.Authenticated);
            var processId = host.Snapshot.ProcessId;
            Assert.IsTrue((await host.StartAsync(request)).IsSuccess);
            Assert.AreEqual(processId, host.Snapshot.ProcessId);
        }
        finally
        {
            await host.DisposeAsync();
        }
        Assert.IsFalse(host.Snapshot.Authenticated);
        Assert.IsNull(host.Snapshot.ProcessId);
        Assert.AreEqual(WindowsDouyinProbeHostState.Closed, host.Snapshot.State);
    }

    [TestMethod]
    public async Task Reopened_room_has_a_new_connection_deadline_and_ignores_old_timer_callback()
    {
        using var fixture = ProbeFixture.Create();
        var time = new StartupTimeProvider();
        await using var host = fixture.CreateHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, reopenMode: "connecting"), time);
        var listening = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var failed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening) listening.TrySetResult();
            if (snapshot.State == WindowsDouyinProbeHostState.Failed) failed.TrySetResult();
        };
        var request = fixture.Request with { Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson };
        Assert.IsTrue((await host.StartAsync(request)).IsSuccess);
        await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.IsTrue((await host.DisconnectAsync()).IsSuccess);
        Assert.IsTrue((await host.StartAsync(request)).IsSuccess);
        Assert.AreEqual(DouyinLiveState.RoomResolved, host.Snapshot.Douyin.State);
        Assert.IsFalse(time.StartupTimerDisposed);
        time.FireFirstQueuedCallback();
        Assert.IsFalse(time.StartupTimerDisposed, "上一房间排队的超时回调不得取消新房间");
        time.AdvanceBeyondStartupDeadline();
        await failed.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.IsFalse(host.Snapshot.Authenticated);
    }

    [TestMethod]
    public async Task Expired_authentication_discards_reused_process_and_next_start_requires_fresh_login()
    {
        using var fixture = ProbeFixture.Create();
        var processStarts = 0;
        await using var host = fixture.CreateHost(new DouyinLiveManager(), _ =>
        {
            processStarts++;
            return CreateCanonicalSidecarStartInfo(fixture.Root, reopenMode: "auth_expired");
        });
        var initial = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var fresh = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var failed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening)
            {
                if (processStarts == 1) initial.TrySetResult(); else fresh.TrySetResult();
            }
            if (snapshot.State == WindowsDouyinProbeHostState.Failed) failed.TrySetResult();
        };
        var request = fixture.Request with { Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson };
        Assert.IsTrue((await host.StartAsync(request)).IsSuccess);
        await initial.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.IsTrue((await host.DisconnectAsync()).IsSuccess);
        Assert.IsFalse((await host.StartAsync(request)).IsSuccess);
        await failed.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.IsFalse(host.Snapshot.Authenticated);
        Assert.AreEqual(WindowsDouyinLoginClearReason.AuthenticationExpired, host.Snapshot.LoginClearReason);
        Assert.IsTrue((await host.StartAsync(request with { QrOutputPath = Path.Combine(fixture.Root, "fresh-qr.png") })).IsSuccess);
        await fresh.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.AreEqual(2, processStarts);
        Assert.IsTrue(host.Snapshot.Authenticated);
    }

    [TestMethod]
    [DataRow(true)]
    [DataRow(false)]
    public async Task Disconnect_timeout_keeps_login_only_with_current_closed_event(bool closedEvent)
    {
        using var fixture = ProbeFixture.Create();
        var time = new StartupTimeProvider();
        await using var host = fixture.CreateHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, closeMode: closedEvent ? "event_only" : "no_response"), time);
        var listening = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var observed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening) listening.TrySetResult();
            if (snapshot.LastEvent == "live_close_observed") observed.TrySetResult();
        };
        var request = fixture.Request with { Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson };
        Assert.AreEqual(WindowsDouyinLoginClearReason.NotAuthenticatedThisRun, host.Snapshot.LoginClearReason);
        Assert.IsTrue((await host.StopAsync()).IsSuccess);
        Assert.AreEqual(WindowsDouyinLoginClearReason.NotAuthenticatedThisRun, host.Snapshot.LoginClearReason);
        Assert.IsTrue((await host.StartAsync(request)).IsSuccess);
        await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));
        var processId = host.Snapshot.ProcessId;
        time.CommandTimerCreated = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var disconnecting = host.DisconnectAsync();
        await time.CommandTimerCreated.Task.WaitAsync(TimeSpan.FromSeconds(5));
        if (closedEvent) await observed.Task.WaitAsync(TimeSpan.FromSeconds(5));
        time.FireCommandDeadline();
        var result = await disconnecting.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.AreEqual(closedEvent, result.IsSuccess);
        Assert.AreEqual(closedEvent, host.Snapshot.Authenticated);
        Assert.AreEqual(closedEvent ? WindowsDouyinLoginClearReason.None : WindowsDouyinLoginClearReason.LocalCommunicationError,
            host.Snapshot.LoginClearReason);
        if (closedEvent)
        {
            Assert.AreEqual(processId, host.Snapshot.ProcessId);
            Assert.AreEqual(WindowsDouyinProbeHostState.Ready, host.Snapshot.State);
        }
        else Assert.AreNotEqual(WindowsDouyinProbeHostState.Ready, host.Snapshot.State);
    }

    [TestMethod]
    [DataRow("expired", WindowsDouyinLoginClearReason.LoginFailed)]
    [DataRow("failed", WindowsDouyinLoginClearReason.LoginFailed)]
    [DataRow("cancelled", WindowsDouyinLoginClearReason.ExplicitStop)]
    public async Task Qr_failure_is_not_reported_as_expired_authenticated_session(string state, WindowsDouyinLoginClearReason expected)
    {
        using var fixture = ProbeFixture.Create();
        await using var host = fixture.CreateHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, authMode: state));
        var failed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.State == WindowsDouyinProbeHostState.Failed) failed.TrySetResult();
        };
        await host.StartAsync(fixture.Request with { Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson });
        await failed.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.IsFalse(host.Snapshot.Authenticated);
        Assert.AreEqual(expected, host.Snapshot.LoginClearReason);
    }

    [TestMethod]
    public async Task Authentication_expired_during_disconnect_overrides_observed_closed_event()
    {
        using var fixture = ProbeFixture.Create();
        await using var host = fixture.CreateHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, closeMode: "closed_then_expired"));
        var listening = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var failed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var sawReady = false;
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening) listening.TrySetResult();
            if (snapshot.State == WindowsDouyinProbeHostState.Ready && snapshot.Authenticated) sawReady = true;
            if (snapshot.State == WindowsDouyinProbeHostState.Failed) failed.TrySetResult();
        };
        Assert.IsTrue((await host.StartAsync(fixture.Request with { Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson })).IsSuccess);
        await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.IsFalse((await host.DisconnectAsync()).IsSuccess);
        await failed.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.IsFalse(sawReady);
        Assert.IsFalse(host.Snapshot.Authenticated);
        Assert.AreEqual(WindowsDouyinLoginClearReason.AuthenticationExpired, host.Snapshot.LoginClearReason);
    }

    [TestMethod]
    [DataRow("room_error", true)]
    [DataRow("no_response", false)]
    public async Task Reopen_room_failure_preserves_login_but_unconfirmed_ipc_does_not(string mode, bool keepsLogin)
    {
        using var fixture = ProbeFixture.Create();
        var time = new StartupTimeProvider();
        await using var host = fixture.CreateHost(new DouyinLiveManager(),
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, reopenMode: mode), time);
        var listening = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening) listening.TrySetResult();
        };
        var request = fixture.Request with { Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson };
        Assert.IsTrue((await host.StartAsync(request)).IsSuccess);
        await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));
        var processId = host.Snapshot.ProcessId;
        Assert.IsTrue((await host.DisconnectAsync()).IsSuccess);
        time.CommandTimerCreated = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var reopening = host.StartAsync(request);
        if (!keepsLogin)
        {
            await time.CommandTimerCreated.Task.WaitAsync(TimeSpan.FromSeconds(5));
            time.FireCommandDeadline();
        }
        Assert.IsFalse((await reopening.WaitAsync(TimeSpan.FromSeconds(5))).IsSuccess);
        Assert.AreEqual(keepsLogin, host.Snapshot.Authenticated);
        Assert.AreEqual(keepsLogin ? WindowsDouyinLoginClearReason.None : WindowsDouyinLoginClearReason.LocalCommunicationError,
            host.Snapshot.LoginClearReason);
        if (keepsLogin)
        {
            Assert.AreEqual(processId, host.Snapshot.ProcessId);
            Assert.AreEqual(WindowsDouyinProbeHostState.Ready, host.Snapshot.State);
        }
        else Assert.AreNotEqual(WindowsDouyinProbeHostState.Ready, host.Snapshot.State);
    }

    private sealed class StartupTimeProvider : TimeProvider
    {
        private StartupTimer? _timer;
        private StartupTimer? _firstTimer;
        private StartupTimer? _commandTimer;
        internal TaskCompletionSource CommandTimerCreated { get; set; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        internal void FireCommandDeadline() => _commandTimer?.Fire();
        internal bool StartupTimerDisposed => _timer?.Disposed == true;
        internal void AdvanceBeyondStartupDeadline() => _timer?.Fire();
        internal void FireFirstQueuedCallback() => _firstTimer?.FireQueued();
        public override ITimer CreateTimer(TimerCallback callback, object? state, TimeSpan dueTime, TimeSpan period)
        {
            if (dueTime < TimeSpan.FromSeconds(30))
            {
                _commandTimer = new StartupTimer(callback, state);
                CommandTimerCreated.TrySetResult();
                return _commandTimer;
            }
            _timer = new StartupTimer(callback, state);
            _firstTimer ??= _timer;
            return _timer;
        }
        private sealed class StartupTimer(TimerCallback callback, object? state) : ITimer
        {
            internal bool Disposed { get; private set; }
            internal void FireQueued() => callback(state);
            internal void Fire()
            {
                if (!Disposed) callback(state);
            }
            public bool Change(TimeSpan dueTime, TimeSpan period) => !Disposed;
            public void Dispose() => Disposed = true;
            public ValueTask DisposeAsync() { Dispose(); return ValueTask.CompletedTask; }
        }
    }

    private static ProcessStartInfo CreateCanonicalSidecarStartInfo(
        string workingDirectory,
        bool emitChat = false,
        bool emitGap = false,
        string? gracefulMarkerPath = null,
        bool emitBulkChat = false,
        string? reopenMode = null,
        bool naturalClose = false,
        string manualSendMode = "accepted",
        bool emitSecondChat = false,
        string? closeMode = null,
        string? authMode = null,
        bool emitDiagnostics = false)
    {
        var powershell = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.Windows),
            "System32",
            "WindowsPowerShell",
            "v1.0",
            "powershell.exe");
        if (!File.Exists(powershell))
        {
            throw new AssertFailedException("测试环境缺少 Windows PowerShell，无法执行 canonical sidecar 管道测试。");
        }

        const string script = """
            [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
            $line = [Console]::In.ReadLine()
            if ($null -eq $line) { exit 2 }
            $qr = $line | ConvertFrom-Json
            if ($qr.op -ne 'auth.qr.start') { exit 3 }
            $qrId = [string]$qr.id
            [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $qrId + '","ok":true,"result":{}}')
            if ($env:GPAUTOLIVE_FAKE_DIAGNOSTICS -eq '1') {
                [Console]::Error.WriteLine('do-not-save-secret-stderr-token-cookie')
                [Console]::Out.WriteLine('{"v":1,"type":"event","event":"auth.diagnostic","payload":{"stage":"qr_fetch","code":"verification_required","http_status":403}}')
                for ($i = 0; $i -lt 20; $i++) {
                    [Console]::Out.WriteLine('{"v":1,"type":"event","event":"auth.diagnostic","payload":{"stage":"qr_fetch","code":"internal_error","message":"do-not-save-secret-response-token"}}')
                }
            }
            if ($env:GPAUTOLIVE_FAKE_AUTH_MODE) {
                [Console]::Out.WriteLine('{"v":1,"type":"event","event":"auth.state","payload":{"state":"' + $env:GPAUTOLIVE_FAKE_AUTH_MODE + '"}}')
                [Console]::Out.Flush()
                [Console]::In.ReadLine() | Out-Null
                exit 0
            }
            [Console]::Out.WriteLine('{"v":1,"type":"event","event":"auth.qr","payload":{"png_base64":"iVBORw0KGgo=","expires_at_unix_ms":' + ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() + 60000) + '}}')
            [Console]::Out.WriteLine('{"v":1,"type":"event","event":"auth.state","payload":{"state":"confirmed"}}')
            [Console]::Out.Flush()

            $line = [Console]::In.ReadLine()
            if ($null -eq $line) { exit 4 }
            $open = $line | ConvertFrom-Json
            if ($open.op -ne 'live.open') { exit 5 }
            $openId = [string]$open.id
            $generation = [string]$open.payload.generation
            [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $openId + '","ok":true,"result":{"session_id":"ls-' + $generation + '","title":"测试直播","live_status":"connected"}}')
            [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.state","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"state":"connected"}}')
            [Console]::Out.Flush()
            if ($env:GPAUTOLIVE_FAKE_NATURAL_CLOSE -eq '1') {
                [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.state","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"state":"closed"}}')
                [Console]::Out.Flush()
            }
            if ($env:GPAUTOLIVE_FAKE_BULK_CHAT -eq '1') {
                $content = 'x' * 1024
                for ($i = 0; $i -lt 600; $i++) {
                    [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.chat","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"msg_id":"bulk-' + $i + '","received_at_unix_ms":0,"author_id":"author-1","nickname":"观众","content":"' + $content + '"}}')
                }
                [Console]::Out.Flush()
            }
            if ($env:GPAUTOLIVE_FAKE_GAP -eq '1') {
                [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.gap","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"reason":"reconnect","dropped_count":2}}')
                [Console]::Out.Flush()
            }
            if ($env:GPAUTOLIVE_FAKE_CHAT -eq '1') {
                [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.chat","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"msg_id":"msg-1","received_at_unix_ms":0,"author_id":"author-1","nickname":"观众","content":"hello"}}')
                [Console]::Out.Flush()
                $line = [Console]::In.ReadLine()
                if ($null -eq $line) { exit 6 }
                $send = $line | ConvertFrom-Json
                if ($send.op -ne 'chat.send') { exit 7 }
                $sendId = [string]$send.id
                $actionId = [string]$send.payload.client_action_id
                [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $sendId + '","ok":true,"result":{"state":"accepted","client_action_id":"' + $actionId + '"}}')
                if ($env:GPAUTOLIVE_FAKE_SECOND_CHAT -eq '1') {
                    [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.chat","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"msg_id":"msg-2","received_at_unix_ms":0,"author_id":"author-1","nickname":"观众","content":"second"}}')
                }
                [Console]::Out.Flush()
            }
            while ($null -ne ($line = [Console]::In.ReadLine())) {
                $command = $line | ConvertFrom-Json
                if ($command.op -eq 'live.close') {
                    if ($env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER) { Add-Content -LiteralPath $env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER -Value 'live.close' }
                    $closeId = [string]$command.id
                    if ($env:GPAUTOLIVE_FAKE_CLOSE_MODE -eq 'no_response') { continue }
                    [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.state","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"state":"closed"}}')
                    if ($env:GPAUTOLIVE_FAKE_CLOSE_MODE -eq 'closed_then_expired') {
                        [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.state","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"state":"auth_expired"}}')
                        [Console]::Out.Flush()
                        continue
                    }
                    if ($env:GPAUTOLIVE_FAKE_CLOSE_MODE -eq 'event_only') { [Console]::Out.Flush(); continue }
                    [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $closeId + '","ok":true,"result":{}}')
                    [Console]::Out.Flush()
                }
                elseif ($command.op -eq 'chat.send') {
                    if ($env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER) { Add-Content -LiteralPath $env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER -Value 'chat.send' }
                    $sendId = [string]$command.id
                    $actionId = [string]$command.payload.client_action_id
                    if ($env:GPAUTOLIVE_FAKE_MANUAL_MODE -eq 'hold') {
                        [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.gap","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"reason":"reconnect","dropped_count":1}}')
                    }
                    elseif ($env:GPAUTOLIVE_FAKE_MANUAL_MODE -eq 'unknown') {
                        [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $sendId + '","ok":false,"error":{"code":"transport_after_dispatch","message":"unknown","retryable":false,"outcome":"unknown"}}')
                    }
                    else {
                        [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $sendId + '","ok":true,"result":{"state":"accepted","client_action_id":"' + $actionId + '"}}')
                        $echo = @{ v=1; type='event'; event='live.chat'; session_id=('ls-' + $generation); generation=[UInt64]$generation; payload=@{ msg_id=('self-' + $actionId); received_at_unix_ms=0; author_id='self'; nickname='我'; content=$command.payload.content; is_self=$true } }
                        [Console]::Out.WriteLine(($echo | ConvertTo-Json -Compress -Depth 4))
                    }
                    [Console]::Out.Flush()
                }
                elseif ($command.op -eq 'live.open') {
                    $oldGeneration = $generation
                    $generation = [string]$command.payload.generation
                    $openId = [string]$command.id
                    if ($env:GPAUTOLIVE_FAKE_REOPEN_MODE -eq 'no_response') { continue }
                    if ($env:GPAUTOLIVE_FAKE_REOPEN_MODE -eq 'room_error') {
                        [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $openId + '","ok":false,"error":{"code":"room_not_live","message":"unavailable","retryable":false,"outcome":"not_sent"}}')
                        [Console]::Out.Flush()
                        continue
                    }
                    if ($env:GPAUTOLIVE_FAKE_REOPEN_MODE -eq 'auth_expired') {
                        [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $openId + '","ok":false,"error":{"code":"auth_expired","message":"expired","retryable":false,"outcome":"not_sent"}}')
                        [Console]::Out.Flush()
                        continue
                    }
                    if ($env:GPAUTOLIVE_FAKE_REOPEN_MODE -eq 'connecting') {
                        [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $openId + '","ok":true,"result":{"session_id":"ls-' + $generation + '","title":"测试直播","live_status":"connecting"}}')
                        [Console]::Out.Flush()
                        continue
                    }
                    $oldMessage = '{"v":1,"type":"event","event":"live.chat","session_id":"ls-' + $oldGeneration + '","generation":' + $oldGeneration + ',"payload":{"msg_id":"retired","received_at_unix_ms":0,"author_id":"author-1","nickname":"观众","content":"old","room_id":"12345"}}'
                    for ($i = 0; $i -lt 20; $i++) { [Console]::Out.WriteLine($oldMessage) }
                    [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $openId + '","ok":true,"result":{"session_id":"ls-' + $generation + '","title":"测试直播","live_status":"connected"}}')
                    [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.state","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"state":"connected"}}')
                    for ($i = 0; $i -lt 20; $i++) { [Console]::Out.WriteLine($oldMessage) }
                    [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.chat","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"msg_id":"fresh","received_at_unix_ms":0,"author_id":"author-1","nickname":"观众","content":"new"}}')
                    [Console]::Out.Flush()
                }
                elseif ($command.op -eq 'shutdown') {
                    if ($env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER) { Add-Content -LiteralPath $env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER -Value 'shutdown' }
                    $shutdownId = [string]$command.id
                    [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $shutdownId + '","ok":true,"result":{}}')
                    [Console]::Out.Flush()
                    exit 0
                }
                elseif ($command.op -eq 'auth.logout') {
                    if ($env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER) { Add-Content -LiteralPath $env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER -Value 'auth.logout' }
                    $logoutId = [string]$command.id
                    [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $logoutId + '","ok":true,"result":{}}')
                    [Console]::Out.Flush()
                }
            }
            """;
        var startInfo = new ProcessStartInfo
        {
            FileName = powershell,
            WorkingDirectory = workingDirectory,
            UseShellExecute = false,
            CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            RedirectStandardInput = true,
            StandardOutputEncoding = new UTF8Encoding(false),
            StandardErrorEncoding = new UTF8Encoding(false),
            StandardInputEncoding = new UTF8Encoding(false)
        };
        startInfo.ArgumentList.Add("-NoProfile");
        startInfo.ArgumentList.Add("-NonInteractive");
        startInfo.ArgumentList.Add("-Command");
        startInfo.ArgumentList.Add(script);
        if (emitChat)
        {
            startInfo.Environment["GPAUTOLIVE_FAKE_CHAT"] = "1";
        }
        if (reopenMode is not null) startInfo.Environment["GPAUTOLIVE_FAKE_REOPEN_MODE"] = reopenMode;
        if (naturalClose) startInfo.Environment["GPAUTOLIVE_FAKE_NATURAL_CLOSE"] = "1";
        if (closeMode is not null) startInfo.Environment["GPAUTOLIVE_FAKE_CLOSE_MODE"] = closeMode;
        if (authMode is not null) startInfo.Environment["GPAUTOLIVE_FAKE_AUTH_MODE"] = authMode;
        if (emitDiagnostics) startInfo.Environment["GPAUTOLIVE_FAKE_DIAGNOSTICS"] = "1";
        startInfo.Environment["GPAUTOLIVE_FAKE_MANUAL_MODE"] = manualSendMode;
        if (emitSecondChat) startInfo.Environment["GPAUTOLIVE_FAKE_SECOND_CHAT"] = "1";
        if (emitBulkChat)
        {
            startInfo.Environment["GPAUTOLIVE_FAKE_BULK_CHAT"] = "1";
        }
        if (emitGap)
        {
            startInfo.Environment["GPAUTOLIVE_FAKE_GAP"] = "1";
        }
        if (!string.IsNullOrWhiteSpace(gracefulMarkerPath))
        {
            startInfo.Environment["GPAUTOLIVE_FAKE_GRACEFUL_MARKER"] = gracefulMarkerPath;
        }

        return startInfo;
    }

    private sealed class ProbeFixture : IDisposable
    {
        internal WindowsDouyinProbeHost CreateHost(DouyinLiveManager manager,
            Func<ProcessStartInfo, ProcessStartInfo>? processStartInfoFactory = null, TimeProvider? timeProvider = null)
            => new(manager, processStartInfoFactory, timeProvider, new WindowsDouyinDiagnosticLog(Path.Combine(Root, "logs")));

        private ProbeFixture(string root, string script, string qr)
        {
            Root = root;
            Request = new WindowsDouyinProbeLaunchRequest(
                Root,
                script,
                Environment.ProcessPath!,
                "gpautolive-douyin",
                qr,
                new DouyinLiveConfig
                {
                    Enabled = true,
                    RoomId = "12345",
                    Replies = ["收到"],
                    QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
                },
                TimeSpan.FromSeconds(30));
        }

        public string Root { get; }
        public WindowsDouyinProbeLaunchRequest Request { get; }

        public static ProbeFixture Create()
        {
            var root = Path.Combine(Path.GetTempPath(), "gpautolive-douyin-host-tests", Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(Path.Combine(root, "builder"));
            Directory.CreateDirectory(Path.Combine(root, "dy_live"));
            Directory.CreateDirectory(Path.Combine(root, "static"));
            File.WriteAllText(Path.Combine(root, "builder", "auth.py"), "# fixture");
            File.WriteAllText(Path.Combine(root, "dy_live", "server.py"), "# fixture");
            File.WriteAllText(Path.Combine(root, "static", "Live_pb2.py"), "# fixture");
            var script = Path.Combine(root, "probe.py");
            File.WriteAllText(script, "# fixture");
            var qr = Path.Combine(root, "probe.png");
            return new(root, script, qr);
        }

        public void Dispose()
        {
            for (var attempt = 0; attempt < 10 && Directory.Exists(Root); attempt++)
            {
                try
                {
                    Directory.Delete(Root, recursive: true);
                }
                catch (IOException) when (attempt < 9)
                {
                    Thread.Sleep(TimeSpan.FromMilliseconds(50));
                }
                catch (UnauthorizedAccessException) when (attempt < 9)
                {
                    Thread.Sleep(TimeSpan.FromMilliseconds(50));
                }
            }
        }
    }
}
