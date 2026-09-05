using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsSystemSpeechAdapterTests
{
    [TestMethod]
    public async Task Speak_marks_playing_after_bridge_start_and_snapshot_contains_no_text()
    {
        var bridge = new FakeBridge();
        await using var adapter = new WindowsSystemSpeechAdapter(
            bridge,
            TimeSpan.FromSeconds(1));
        var voiceKey = TestVoiceKey;

        var speakTask = adapter.SpeakAsync(
            FixedSpeechCommandDto.Speak("speech-1", "不应进入快照的正文"),
            voiceKey);
        var operation = await bridge.WaitForOperationAsync(0);
        operation.MarkStarted();

        var accepted = await speakTask;

        Assert.IsTrue(accepted.IsAccepted);
        Assert.AreEqual(WindowsSpeechAdapterState.Playing, accepted.Snapshot.State);
        Assert.AreEqual(voiceKey, accepted.Snapshot.VoiceKey);
        Assert.IsFalse(accepted.Snapshot.Error?.Contains("不应进入快照的正文", StringComparison.Ordinal) == true);
        Assert.IsNull(typeof(WindowsSpeechAdapterSnapshot).GetProperty("Text"));

        operation.MarkCompleted();
        var terminal = await accepted.Completion!.WaitAsync(TimeSpan.FromSeconds(1));
        Assert.IsTrue(terminal.IsSuccess);
        Assert.AreEqual(WindowsSpeechAdapterState.Completed, terminal.Snapshot.State);
    }

    [TestMethod]
    public async Task Cancel_stops_and_releases_the_single_active_operation()
    {
        var bridge = new FakeBridge();
        await using var adapter = new WindowsSystemSpeechAdapter(bridge);
        var speakTask = adapter.SpeakAsync(FixedSpeechCommandDto.Speak("speech-2", "欢迎"));
        var operation = await bridge.WaitForOperationAsync(0);
        operation.MarkStarted();
        var accepted = await speakTask;

        var cancelled = await adapter.CancelAsync("speech-2");

        Assert.IsFalse(cancelled.IsAccepted);
        Assert.AreEqual(WindowsSpeechAdapterState.Cancelled, cancelled.Snapshot.State);
        Assert.AreEqual(1, operation.CancelCount);
        Assert.AreEqual(1, operation.DisposeCount);
        var terminal = await accepted.Completion!.WaitAsync(TimeSpan.FromSeconds(1));
        Assert.AreEqual(WindowsSpeechAdapterState.Cancelled, terminal.Snapshot.State);
    }

    [TestMethod]
    public async Task Startup_timeout_fails_closed_and_releases_operation()
    {
        var bridge = new FakeBridge();
        await using var adapter = new WindowsSystemSpeechAdapter(
            bridge,
            TimeSpan.FromMilliseconds(25));

        var result = await adapter.SpeakAsync(FixedSpeechCommandDto.Speak("speech-3", "超时"));

        Assert.IsFalse(result.IsAccepted);
        Assert.AreEqual(WindowsSpeechFailureCode.StartupTimeout, result.Error?.Code);
        Assert.AreEqual(WindowsSpeechAdapterState.Failed, result.Snapshot.State);
        var operation = await bridge.WaitForOperationAsync(0);
        Assert.AreEqual(1, operation.CancelCount);
        Assert.AreEqual(1, operation.DisposeCount);
    }

    [TestMethod]
    public async Task New_speak_cancels_old_operation_and_late_completion_is_ignored()
    {
        var bridge = new FakeBridge();
        await using var adapter = new WindowsSystemSpeechAdapter(bridge);

        var firstSpeakTask = adapter.SpeakAsync(FixedSpeechCommandDto.Speak("speech-old", "旧话术"));
        var first = await bridge.WaitForOperationAsync(0);
        first.MarkStarted();
        var firstAccepted = await firstSpeakTask;

        var secondSpeakTask = adapter.SpeakAsync(FixedSpeechCommandDto.Speak("speech-new", "新话术"));
        var second = await bridge.WaitForOperationAsync(1);
        second.MarkStarted();
        var secondAccepted = await secondSpeakTask;

        Assert.IsTrue(secondAccepted.IsAccepted);
        Assert.AreEqual("speech-new", adapter.Snapshot.OperationId);
        Assert.AreEqual(WindowsSpeechAdapterState.Playing, adapter.Snapshot.State);
        Assert.AreEqual(1, first.CancelCount);
        var firstTerminal = await firstAccepted.Completion!.WaitAsync(TimeSpan.FromSeconds(1));
        Assert.AreEqual(WindowsSpeechAdapterState.Cancelled, firstTerminal.Snapshot.State);

        first.MarkCompleted();
        Assert.AreEqual("speech-new", adapter.Snapshot.OperationId);
        Assert.AreEqual(WindowsSpeechAdapterState.Playing, adapter.Snapshot.State);

        second.MarkCompleted();
        var secondTerminal = await secondAccepted.Completion!.WaitAsync(TimeSpan.FromSeconds(1));
        Assert.AreEqual(WindowsSpeechAdapterState.Completed, secondTerminal.Snapshot.State);
    }

    [TestMethod]
    public async Task Microphone_priority_rejects_without_touching_bridge()
    {
        var bridge = new FakeBridge();
        await using var adapter = new WindowsSystemSpeechAdapter(bridge);

        var result = await adapter.SpeakAsync(
            FixedSpeechCommandDto.Speak("speech-mic", "麦克风优先"),
            microphonePriorityActive: true);

        Assert.IsFalse(result.IsAccepted);
        Assert.AreEqual(WindowsSpeechFailureCode.MicrophonePriority, result.Error?.Code);
        Assert.AreEqual(WindowsSpeechAdapterState.Idle, result.Snapshot.State);
        Assert.AreEqual(0, bridge.StartCount);
    }

    [TestMethod]
    public async Task Raw_voice_token_is_rejected_at_adapter_boundary()
    {
        var bridge = new FakeBridge();
        await using var adapter = new WindowsSystemSpeechAdapter(bridge);

        var result = await adapter.SpeakAsync(
            FixedSpeechCommandDto.Speak("speech-voice", "本地语音"),
            voiceKey: "SAPI\\Token\\Secret");

        Assert.IsFalse(result.IsAccepted);
        Assert.AreEqual(WindowsSpeechFailureCode.InvalidCommand, result.Error?.Code);
        Assert.AreEqual(0, bridge.StartCount);
    }

    [TestMethod]
    public async Task Bridge_start_error_is_redacted_and_does_not_leave_active_state()
    {
        var bridge = new FakeBridge
        {
            StartError = new(
                WindowsSpeechFailureCode.SapiUnavailable,
                "SAPI unavailable")
        };
        await using var adapter = new WindowsSystemSpeechAdapter(bridge);

        var speakTask = adapter.SpeakAsync(FixedSpeechCommandDto.Speak("speech-fail", "失败"));
        var operation = await bridge.WaitForOperationAsync(0);
        operation.MarkStarted(success: false, bridge.StartError);
        var result = await speakTask;

        Assert.IsFalse(result.IsAccepted);
        Assert.AreEqual(WindowsSpeechFailureCode.SapiUnavailable, result.Error?.Code);
        Assert.AreEqual(WindowsSpeechAdapterState.Failed, result.Snapshot.State);
        Assert.AreEqual("speech-fail", adapter.Snapshot.OperationId);
        Assert.AreEqual(1, operation.DisposeCount);
    }

    [TestMethod]
    public async Task Dispose_closes_adapter_and_com_operation_within_boundary()
    {
        var bridge = new FakeBridge();
        var adapter = new WindowsSystemSpeechAdapter(bridge);
        var speakTask = adapter.SpeakAsync(FixedSpeechCommandDto.Speak("speech-dispose", "退出"));
        var operation = await bridge.WaitForOperationAsync(0);
        operation.MarkStarted();
        var accepted = await speakTask;

        await adapter.DisposeAsync();

        Assert.AreEqual(WindowsSpeechAdapterState.Closed, adapter.Snapshot.State);
        Assert.AreEqual(1, operation.CancelCount);
        Assert.AreEqual(1, operation.DisposeCount);
        Assert.AreEqual(1, bridge.DisposeCount);
        Assert.AreEqual(WindowsSpeechAdapterState.Closed, (await accepted.Completion!.WaitAsync(TimeSpan.FromSeconds(1))).Snapshot.State);
    }

    [TestMethod]
    public async Task Voice_catalog_exposes_only_redacted_descriptor()
    {
        var bridge = new FakeBridge();
        await using var adapter = new WindowsSystemSpeechAdapter(bridge);

        var result = adapter.GetVoices();

        Assert.IsTrue(result.IsAvailable);
        Assert.AreEqual(1, result.Voices.Count);
        Assert.AreEqual("本地系统语音", result.Voices[0].DisplayName);
        Assert.IsFalse(result.Voices[0].Key.Contains("SAPI", StringComparison.OrdinalIgnoreCase));
    }

    private sealed class FakeBridge : IWindowsSpeechBridge
    {
        public TaskCompletionSource<FakeOperation> OperationCreated { get; private set; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        public List<FakeOperation> Operations { get; } = [];
        public WindowsSpeechError? StartError { get; init; }
        public int StartCount { get; private set; }
        public int DisposeCount { get; private set; }

        public WindowsSpeechVoiceCatalogResult GetVoices() =>
            WindowsSpeechVoiceCatalogResult.Available([
                new(TestVoiceKey, "zh-CN")
            ]);

        public Task<IWindowsSpeechOperation> StartAsync(
            string text,
            string? voiceKey,
            CancellationToken cancellationToken = default)
        {
            StartCount++;
            var operation = new FakeOperation(StartError);
            Operations.Add(operation);
            OperationCreated.TrySetResult(operation);
            OperationCreated = new(TaskCreationOptions.RunContinuationsAsynchronously);
            return Task.FromResult<IWindowsSpeechOperation>(operation);
        }

        public async Task<FakeOperation> WaitForOperationAsync(int index)
        {
            while (Operations.Count <= index)
            {
                await Task.Delay(1).ConfigureAwait(false);
            }

            return Operations[index];
        }

        public ValueTask DisposeAsync()
        {
            DisposeCount++;
            return ValueTask.CompletedTask;
        }
    }

    private sealed class FakeOperation(WindowsSpeechError? startError) : IWindowsSpeechOperation
    {
        private readonly TaskCompletionSource<WindowsSpeechBridgeStartResult> _started =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly TaskCompletionSource<WindowsSpeechBridgeCompletion> _completion =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        public Task<WindowsSpeechBridgeStartResult> Started => _started.Task;
        public Task<WindowsSpeechBridgeCompletion> Completion => _completion.Task;
        public int CancelCount { get; private set; }
        public int DisposeCount { get; private set; }

        public void MarkStarted(bool success = true, WindowsSpeechError? error = null) =>
            _started.TrySetResult(new(success, error ?? startError));

        public void MarkCompleted() =>
            _completion.TrySetResult(new(WindowsSpeechBridgeCompletionKind.Completed));

        public Task CancelAsync(CancellationToken cancellationToken = default)
        {
            CancelCount++;
            _started.TrySetResult(new(
                false,
                new(WindowsSpeechFailureCode.Cancelled, "cancelled", Retryable: true)));
            _completion.TrySetResult(new(WindowsSpeechBridgeCompletionKind.Cancelled));
            return Task.CompletedTask;
        }

        public ValueTask DisposeAsync()
        {
            DisposeCount++;
            return ValueTask.CompletedTask;
        }
    }

    private static string TestVoiceKey => new('A', 64);
}
