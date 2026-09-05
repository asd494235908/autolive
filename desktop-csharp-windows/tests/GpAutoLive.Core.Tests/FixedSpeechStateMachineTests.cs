using GpAutoLive.Contracts;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class FixedSpeechStateMachineTests
{
    [TestMethod]
    public void Valid_operation_moves_starting_to_playing_and_completed()
    {
        var machine = new FixedSpeechStateMachine();

        var started = machine.BeginSpeak(FixedSpeechCommandDto.Speak("speech-1", "欢迎"));
        var playing = machine.MarkPlaying("speech-1");
        var completed = machine.Complete("speech-1");

        Assert.AreEqual(FixedSpeechTransitionKind.Accepted, started.Kind);
        Assert.AreEqual(FixedSpeechRuntimeState.Starting, started.Snapshot.State);
        Assert.AreEqual(FixedSpeechTransitionKind.Accepted, playing.Kind);
        Assert.AreEqual(FixedSpeechRuntimeState.Playing, playing.Snapshot.State);
        Assert.AreEqual(FixedSpeechTransitionKind.Completed, completed.Kind);
        Assert.AreEqual(FixedSpeechRuntimeState.Completed, machine.Snapshot.State);
        Assert.AreEqual("speech-1", machine.Snapshot.OperationId);
        Assert.IsNull(machine.Snapshot.Error);
    }

    [TestMethod]
    public void New_speech_supersedes_active_operation_without_accepting_stale_callbacks()
    {
        var machine = new FixedSpeechStateMachine();
        machine.BeginSpeak(FixedSpeechCommandDto.Speak("speech-old", "旧话术"));
        machine.MarkPlaying("speech-old");

        var replacement = machine.BeginSpeak(FixedSpeechCommandDto.Speak("speech-new", "新话术"));
        var staleComplete = machine.Complete("speech-old");

        Assert.IsTrue(replacement.IsAccepted);
        Assert.AreEqual("speech-old", replacement.SupersededOperationId);
        Assert.AreEqual("speech-new", replacement.Snapshot.OperationId);
        Assert.AreEqual(FixedSpeechRuntimeState.Starting, replacement.Snapshot.State);
        Assert.AreEqual(FixedSpeechTransitionKind.Ignored, staleComplete.Kind);
        Assert.AreEqual(FixedSpeechRuntimeState.Starting, machine.Snapshot.State);
        Assert.AreEqual("speech-new", machine.Snapshot.OperationId);
    }

    [TestMethod]
    public void Microphone_priority_cancels_incoming_speech_and_preserves_current_state()
    {
        var machine = new FixedSpeechStateMachine();
        machine.BeginSpeak(FixedSpeechCommandDto.Speak("speech-current", "正在朗读"));
        machine.MarkPlaying("speech-current");

        var blocked = machine.BeginSpeak(
            FixedSpeechCommandDto.Speak("speech-blocked", "麦克风优先"),
            microphonePriorityActive: true);

        Assert.AreEqual(FixedSpeechTransitionKind.Cancelled, blocked.Kind);
        Assert.AreEqual("speech-blocked", blocked.OperationId);
        Assert.AreEqual(FixedSpeechRuntimeState.Playing, machine.Snapshot.State);
        Assert.AreEqual("speech-current", machine.Snapshot.OperationId);
    }

    [TestMethod]
    public void Stale_cancel_is_ignored_and_active_cancel_is_idempotent()
    {
        var machine = new FixedSpeechStateMachine();
        machine.BeginSpeak(FixedSpeechCommandDto.Speak("speech-1", "有效"));

        var stale = machine.Cancel(FixedSpeechCommandDto.Cancel("speech-other"));
        var cancelled = machine.Cancel(FixedSpeechCommandDto.Cancel("speech-1"));
        var duplicate = machine.Cancel(FixedSpeechCommandDto.Cancel("speech-1"));

        Assert.AreEqual(FixedSpeechTransitionKind.Ignored, stale.Kind);
        Assert.AreEqual(FixedSpeechTransitionKind.Cancelled, cancelled.Kind);
        Assert.AreEqual(FixedSpeechRuntimeState.Cancelled, cancelled.Snapshot.State);
        Assert.AreEqual(FixedSpeechTransitionKind.Ignored, duplicate.Kind);
        Assert.AreEqual(FixedSpeechRuntimeState.Cancelled, machine.Snapshot.State);
    }

    [TestMethod]
    public void Invalid_or_stale_events_do_not_reset_current_snapshot()
    {
        var machine = new FixedSpeechStateMachine();
        machine.BeginSpeak(FixedSpeechCommandDto.Speak("speech-1", "有效"));

        var rejected = machine.BeginSpeak(FixedSpeechCommandDto.Speak("speech-2", " "));
        var staleFailure = machine.Fail("speech-old", "不应覆盖");

        Assert.AreEqual(FixedSpeechTransitionKind.Rejected, rejected.Kind);
        Assert.AreEqual(FixedSpeechRuntimeState.Starting, rejected.Snapshot.State);
        Assert.AreEqual("speech-1", rejected.Snapshot.OperationId);
        Assert.AreEqual(FixedSpeechTransitionKind.Ignored, staleFailure.Kind);
        Assert.AreEqual(FixedSpeechRuntimeState.Starting, machine.Snapshot.State);
        Assert.IsNull(machine.Snapshot.Error);
    }

    [TestMethod]
    public void Failure_keeps_only_bounded_error_summary()
    {
        var machine = new FixedSpeechStateMachine();
        machine.BeginSpeak(FixedSpeechCommandDto.Speak("speech-1", "有效"));

        var failed = machine.Fail("speech-1", string.Concat(Enumerable.Repeat("错", 501)));

        Assert.AreEqual(FixedSpeechTransitionKind.Failed, failed.Kind);
        Assert.AreEqual("系统语音播放失败", failed.Snapshot.Error);
        Assert.AreEqual(FixedSpeechRuntimeState.Failed, machine.Snapshot.State);
    }
}
