namespace GpAutoLive.Core;

/// <summary>本地音频层级；数值越大优先级越高。</summary>
public enum AudioPriorityLayer
{
    /// <summary>原媒体或普通声音。</summary>
    OriginalMedia = 0,
    /// <summary>插话文件。</summary>
    InterludeFile = 1,
    /// <summary>固定话术。</summary>
    FixedSpeech = 2,
    /// <summary>本地麦克风插话。</summary>
    Microphone = 3,
}

/// <summary>音频层级请求的结果类别。</summary>
public enum AudioPriorityDecisionKind
{
    /// <summary>请求已接受。</summary>
    Accepted,
    /// <summary>请求被拒绝。</summary>
    Rejected,
    /// <summary>层级已释放。</summary>
    Released,
    /// <summary>请求不改变当前快照。</summary>
    Ignored,
}

/// <summary>音频层级策略的脱敏快照；不保存音频正文。</summary>
public sealed record AudioPrioritySnapshot(
    AudioPriorityLayer? FocusLayer,
    bool MicrophoneSpeaking,
    bool FixedSpeechActive,
    bool InterludeActive,
    bool MediaMuted,
    bool MediaDucked,
    bool InterludeMuted,
    ulong Generation);

/// <summary>音频层级策略请求结果。</summary>
public sealed record AudioPriorityDecision(
    bool IsAccepted,
    AudioPriorityDecisionKind Kind,
    AudioPrioritySnapshot Snapshot,
    AudioPriorityLayer? PreemptedLayer = null,
    string? Error = null);

/// <summary>
/// 本机音频优先级的唯一纯逻辑所有者。
/// 麦克风插话 > 固定话术 > 插话文件 > 原媒体；被麦克风截断的低优先级内容不自动续播。
/// 该类只产生静音/duck 策略，不直接碰 PortAudio、SAPI 或文件。
/// </summary>
public sealed class AudioPriorityCoordinator
{
    private readonly object _gate = new();
    private AudioPriorityLayer? _focusLayer;
    private bool _microphoneSpeaking;
    private ulong _generation;

    /// <summary>当前层级策略快照。</summary>
    public AudioPrioritySnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshotUnsafe();
            }
        }
    }

    /// <summary>
    /// 请求开始固定话术。麦克风优先标志来自输入所有者，用于在输入流尚未切换快照时 fail-closed。
    /// </summary>
    public AudioPriorityDecision BeginFixedSpeech(bool microphonePriorityActive = false)
    {
        lock (_gate)
        {
            if (microphonePriorityActive || _microphoneSpeaking)
            {
                return RejectUnsafe("麦克风正在插话，固定话术未启动。");
            }

            var preempted = _focusLayer is AudioPriorityLayer.InterludeFile or AudioPriorityLayer.FixedSpeech
                ? _focusLayer
                : null;
            _focusLayer = AudioPriorityLayer.FixedSpeech;
            AdvanceGenerationUnsafe();
            return AcceptUnsafe(preempted);
        }
    }

    /// <summary>请求开始一个插话文件；固定话术或麦克风期间拒绝新插话。</summary>
    public AudioPriorityDecision BeginInterludeFile()
    {
        lock (_gate)
        {
            if (_microphoneSpeaking)
            {
                return RejectUnsafe("麦克风正在插话，插话文件未启动。");
            }

            if (_focusLayer is AudioPriorityLayer.FixedSpeech)
            {
                return RejectUnsafe("固定话术正在播放，插话文件未启动。");
            }

            if (_focusLayer is AudioPriorityLayer.InterludeFile)
            {
                return RejectUnsafe("插话文件已经在播放。");
            }

            _focusLayer = AudioPriorityLayer.InterludeFile;
            AdvanceGenerationUnsafe();
            return AcceptUnsafe();
        }
    }

    /// <summary>结束指定层；不会自动恢复被截断的低优先级内容。</summary>
    public AudioPriorityDecision End(AudioPriorityLayer layer)
    {
        lock (_gate)
        {
            if (layer is AudioPriorityLayer.Microphone)
            {
                return SetMicrophoneSpeakingUnsafe(false);
            }

            if (_focusLayer != layer)
            {
                return new(false, AudioPriorityDecisionKind.Ignored, CreateSnapshotUnsafe());
            }

            _focusLayer = null;
            AdvanceGenerationUnsafe();
            return new(true, AudioPriorityDecisionKind.Released, CreateSnapshotUnsafe());
        }
    }

    /// <summary>
    /// 更新麦克风门控状态。开始说话会立即截断固定话术/插话；结束后回到原媒体且不自动续播。
    /// </summary>
    public AudioPriorityDecision SetMicrophoneSpeaking(bool speaking)
    {
        lock (_gate)
        {
            return SetMicrophoneSpeakingUnsafe(speaking);
        }
    }

    private AudioPriorityDecision SetMicrophoneSpeakingUnsafe(bool speaking)
    {
        if (_microphoneSpeaking == speaking)
        {
            return new(false, AudioPriorityDecisionKind.Ignored, CreateSnapshotUnsafe());
        }

        if (speaking)
        {
            var preempted = _focusLayer is AudioPriorityLayer.FixedSpeech or AudioPriorityLayer.InterludeFile
                ? _focusLayer
                : null;
            _microphoneSpeaking = true;
            _focusLayer = AudioPriorityLayer.Microphone;
            AdvanceGenerationUnsafe();
            return AcceptUnsafe(preempted);
        }

        _microphoneSpeaking = false;
        if (_focusLayer is AudioPriorityLayer.Microphone)
        {
            _focusLayer = null;
        }

        AdvanceGenerationUnsafe();
        return new(true, AudioPriorityDecisionKind.Released, CreateSnapshotUnsafe());
    }

    private AudioPriorityDecision AcceptUnsafe(AudioPriorityLayer? preempted = null) =>
        new(true, AudioPriorityDecisionKind.Accepted, CreateSnapshotUnsafe(), preempted);

    private AudioPriorityDecision RejectUnsafe(string error) =>
        new(false, AudioPriorityDecisionKind.Rejected, CreateSnapshotUnsafe(), Error: error);

    private void AdvanceGenerationUnsafe()
    {
        if (_generation < ulong.MaxValue)
        {
            _generation++;
        }
    }

    private AudioPrioritySnapshot CreateSnapshotUnsafe()
    {
        var fixedSpeechActive = _focusLayer is AudioPriorityLayer.FixedSpeech;
        var interludeActive = _focusLayer is AudioPriorityLayer.InterludeFile;
        var mediaMuted = _microphoneSpeaking || fixedSpeechActive;
        return new(
            _focusLayer,
            _microphoneSpeaking,
            fixedSpeechActive,
            interludeActive,
            mediaMuted,
            !mediaMuted && interludeActive,
            _microphoneSpeaking || fixedSpeechActive,
            _generation);
    }
}
