using System.Runtime.CompilerServices;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>Windows mpv 播放控制器的脱敏状态。</summary>
public enum WindowsMpvPlaybackControllerState
{
    Closed,
    Ready,
    Starting,
    Playing,
    Paused,
    Faulted,
}

/// <summary>播放控制器的稳定错误分类。</summary>
public enum WindowsMpvPlaybackControllerFailureCode
{
    InvalidInput,
    RuntimeUnavailable,
    SourceNotVideo,
    InvalidPlan,
    AlreadyRunning,
    NotRunning,
    DispatchFailed,
    EffectiveFrameNotObserved,
    FirstFrameNotObserved,
    StartFailed,
    StopFailed,
    Cancelled,
    Closed,
}

/// <summary>不携带路径、命令行或 IPC 原始正文的播放控制器错误。</summary>
public sealed record WindowsMpvPlaybackControllerError(
    WindowsMpvPlaybackControllerFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>播放控制器当前状态和活动源身份快照。</summary>
public sealed record WindowsMpvPlaybackControllerSnapshot(
    WindowsMpvPlaybackControllerState State,
    MediaPlaybackIdentity? ActiveIdentity,
    WindowsMpvPlaybackRuntimeSnapshot Runtime);

/// <summary>播放控制器操作结果。</summary>
public sealed record WindowsMpvPlaybackControllerResult(
    bool IsSuccess,
    WindowsMpvPlaybackControllerSnapshot Snapshot,
    WindowsMpvPlaybackControllerError? Error = null);

/// <summary>
/// 把 Core 播放身份、Media 的 mpv 会话和 Windows 受管宿主组合成一个播放所有者。
/// WPF 只传递已验证资源、当前源和 HWND，不负责拼接命令行或管理进程。
/// </summary>
public sealed class WindowsMpvPlaybackController : IAsyncDisposable
{
    private static readonly TimeSpan NextFrameObservationTimeout = TimeSpan.FromSeconds(2);
    private static readonly TimeSpan NextFrameObservationInterval = TimeSpan.FromMilliseconds(50);
    private static readonly TimeSpan SourceReadyObservationTimeout = TimeSpan.FromSeconds(2);
    private static readonly TimeSpan SourceReadyObservationInterval = TimeSpan.FromMilliseconds(50);
    private readonly object _gate = new();
    private readonly SemaphoreSlim _serial = new(1, 1);
    private WindowsMpvPlaybackRuntime? _runtime;
    private MpvPlaybackSession? _session;
    private WindowsMpvPlaybackControllerState _state = WindowsMpvPlaybackControllerState.Ready;
    private MediaPlaybackIdentity? _activeIdentity;
    private bool _disposed;

    public WindowsMpvPlaybackControllerSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                var runtime = _runtime?.Snapshot ?? EmptyRuntimeSnapshot();
                var state = _state;
                if (_runtime is not null
                    && state is (WindowsMpvPlaybackControllerState.Starting
                        or WindowsMpvPlaybackControllerState.Playing
                        or WindowsMpvPlaybackControllerState.Paused)
                    && runtime.State is (WindowsMpvPlaybackRuntimeState.Faulted
                        or WindowsMpvPlaybackRuntimeState.Stopped
                        or WindowsMpvPlaybackRuntimeState.Closed))
                {
                    state = WindowsMpvPlaybackControllerState.Faulted;
                }

                if (_runtime is not null
                    && state is (WindowsMpvPlaybackControllerState.Playing
                        or WindowsMpvPlaybackControllerState.Paused)
                    && (runtime.State is not WindowsMpvPlaybackRuntimeState.Running
                        || runtime.IpcState is not MpvIpcPipeState.Connected))
                {
                    state = WindowsMpvPlaybackControllerState.Faulted;
                }

                return new(
                    state,
                    _activeIdentity,
                    runtime);
            }
        }
    }

    /// <summary>创建 mpv 会话、绑定活动视频源并开始播放。</summary>
    public async Task<WindowsMpvPlaybackControllerResult> StartAsync(
        VerifiedMediaRuntime? resources,
        SourceMediaDto? source,
        MediaPlaybackIdentity? identity,
        uint hostWindowId,
        MpvLaunchMode mode = MpvLaunchMode.Gpu83,
        CancellationToken cancellationToken = default,
        MpvVideoEffectSnapshot? initialEffectSnapshot = null,
        bool waitForFirstFrame = false)
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return Failure(WindowsMpvPlaybackControllerFailureCode.Closed, "mpv 播放控制器已关闭。", false);
            }
        }

        if (source is null || identity is null)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.InvalidInput, "播放源或播放身份不能为空。", false);
        }

        if (source.MediaKind is not MediaKind.Video)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.SourceNotVideo, "只有视频源可以绑定 mpv。", false);
        }

        if (hostWindowId == 0)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.InvalidPlan, "视频表面 HWND 无效。", false);
        }

        if (resources is null)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.RuntimeUnavailable, "媒体运行资源尚未校验。", true);
        }

        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.Cancelled, "mpv 启动已取消。", true);
        }

        try
        {
            if (_disposed)
            {
                return Failure(WindowsMpvPlaybackControllerFailureCode.Closed, "mpv 播放控制器已关闭。", false);
            }

            if (_runtime is not null)
            {
                return Failure(WindowsMpvPlaybackControllerFailureCode.AlreadyRunning, "mpv 播放会话已经创建。", false);
            }

            if (!MpvActiveSource.TryCreate(source, identity, out var activeSource, out var sourceError)
                || activeSource is null)
            {
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.InvalidInput,
                    sourceError?.Message ?? "mpv 活动源无效。",
                    false);
            }

            var pipeName = $"\\\\.\\pipe\\gpautolive-csharp-mpv-{Guid.NewGuid():N}";
            if (!MpvIpcPipeEndpoint.TryCreate(pipeName, out var endpoint, out var pipeError)
                || endpoint is null)
            {
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.InvalidPlan,
                    pipeError?.Message ?? "mpv IPC 端点无效。",
                    false);
            }

            if (!MpvLaunchPlan.TryCreate(
                    resources,
                    source.SourcePath,
                    hostWindowId,
                    endpoint,
                    mode,
                    sourceStartMs: 0,
                    source.DurationMs,
                    out var plan,
                    out var planError)
                || plan is null)
            {
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.InvalidPlan,
                    planError?.Message ?? "mpv 启动计划无效。",
                    false);
            }

            if (!TryCreateInitialEffectSnapshot(mode, initialEffectSnapshot, out var resolvedEffectSnapshot)
                || resolvedEffectSnapshot is null)
            {
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.InvalidPlan,
                    "mpv 初始视频参数快照无效。",
                    false);
            }

            var session = new MpvPlaybackSession();
            if (!session.BindSource(activeSource).IsSuccess)
            {
                return Failure(WindowsMpvPlaybackControllerFailureCode.InvalidInput, "mpv 活动源绑定失败。", false);
            }

            var runtime = new WindowsMpvPlaybackRuntime();
            lock (_gate)
            {
                _state = WindowsMpvPlaybackControllerState.Starting;
                _runtime = runtime;
                _session = session;
                _activeIdentity = identity;
            }

            var started = await runtime.StartAsync(plan, session, cancellationToken: cancellationToken).ConfigureAwait(false);
            if (!started.IsSuccess)
            {
                await runtime.DisposeAsync().ConfigureAwait(false);
                ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.StartFailed,
                    started.Error?.Message ?? "mpv 无法启动。",
                    started.Error?.Retryable ?? true);
            }

            var initialEffects = session.UpdateEffects(identity, resolvedEffectSnapshot);
            var effectsDispatched = await DispatchCommandsAsync(initialEffects, identity, cancellationToken).ConfigureAwait(false);
            if (!effectsDispatched.IsSuccess)
            {
                await runtime.DisposeAsync().ConfigureAwait(false);
                ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                return effectsDispatched;
            }

            if (resolvedEffectSnapshot.Mode is MpvVideoProcessingMode.Cpu4
                or MpvVideoProcessingMode.Gpu83)
            {
                var readbackProperty = resolvedEffectSnapshot.Mode is MpvVideoProcessingMode.Cpu4
                    ? MpvIpcProperty.VideoFilterChain
                    : MpvIpcProperty.ShaderOptions;
                var readback = await runtime.DispatchAsync(
                        MpvIpcCommand.GetProperty(readbackProperty),
                        identity,
                        cancellationToken)
                    .ConfigureAwait(false);
                var readbackFailure = resolvedEffectSnapshot.Mode is MpvVideoProcessingMode.Cpu4
                    ? CreateCpu4ReadbackFailure(readback, resolvedEffectSnapshot)
                    : CreateGpu83ReadbackFailure(readback, resolvedEffectSnapshot.ShaderOptions);
                if (readbackFailure is not null)
                {
                    await runtime.DisposeAsync().ConfigureAwait(false);
                    ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                    return readbackFailure;
                }
            }

            var startOperation = session.Start();
            var dispatched = await DispatchCommandsAsync(startOperation, identity, cancellationToken).ConfigureAwait(false);
            if (!dispatched.IsSuccess)
            {
                await runtime.DisposeAsync().ConfigureAwait(false);
                ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                return dispatched;
            }

            if (waitForFirstFrame)
            {
                var firstFrame = await WaitForFirstFrameAsync(
                        runtime,
                        identity,
                        cancellationToken)
                    .ConfigureAwait(false);
                if (!firstFrame.IsSuccess)
                {
                    await runtime.DisposeAsync().ConfigureAwait(false);
                    ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                    return firstFrame;
                }
            }

            lock (_gate)
            {
                _state = WindowsMpvPlaybackControllerState.Playing;
            }

            return Success();
        }
        catch (OperationCanceledException)
        {
            WindowsMpvPlaybackRuntime? runtime;
            lock (_gate)
            {
                runtime = _runtime;
            }

            if (runtime is not null)
            {
                await runtime.DisposeAsync().ConfigureAwait(false);
            }

            ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
            return Failure(WindowsMpvPlaybackControllerFailureCode.Cancelled, "mpv 启动已取消。", true);
        }
        catch (Exception exception) when (exception is InvalidOperationException
            or System.ComponentModel.Win32Exception
            or IOException
            or UnauthorizedAccessException
            or NotSupportedException
            or ObjectDisposedException)
        {
            WindowsMpvPlaybackRuntime? runtime;
            lock (_gate)
            {
                runtime = _runtime;
            }

            if (runtime is not null)
            {
                await runtime.DisposeAsync().ConfigureAwait(false);
            }

            ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
            return Failure(WindowsMpvPlaybackControllerFailureCode.StartFailed, "mpv 播放会话无法建立。", true);
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>在当前活动身份下暂停或恢复播放。</summary>
    public async Task<WindowsMpvPlaybackControllerResult> TogglePauseAsync(
        MediaPlaybackIdentity? expectedIdentity,
        CancellationToken cancellationToken = default)
    {
        return await RunSessionOperationAsync(
            expectedIdentity,
            session => session.Snapshot.State is MpvSessionState.Playing
                ? session.Pause()
                : session.Snapshot.State is MpvSessionState.Paused or MpvSessionState.Ready
                    ? session.Start()
                    : MpvSessionOperationResult.Failure(session.Snapshot, new(MpvSessionFailureCode.InvalidStateTransition, "mpv 当前状态不能切换播放。")),
            cancellationToken).ConfigureAwait(false);
    }

    /// <summary>停止当前播放但保留活动源，下一次开始无需重新导入媒体。</summary>
    public async Task<WindowsMpvPlaybackControllerResult> StopPlaybackAsync(
        MediaPlaybackIdentity? expectedIdentity,
        CancellationToken cancellationToken = default)
    {
        return await RunSessionOperationAsync(
            expectedIdentity,
            static session => session.Stop(),
            cancellationToken).ConfigureAwait(false);
    }

    /// <summary>在当前活动身份下提交已校验的视频效果快照。</summary>
    public async Task<WindowsMpvPlaybackControllerResult> UpdateEffectsAsync(
        MediaPlaybackIdentity? expectedIdentity,
        MpvVideoEffectSnapshot? next,
        CancellationToken cancellationToken = default,
        bool waitForNextFrame = false)
    {
        var observeNextFrame = waitForNextFrame && IsPlaying(expectedIdentity);
        ulong? previousFrameNumber = null;
        if (observeNextFrame)
        {
            var previousFrame = await ReadPropertyAsync(
                    expectedIdentity,
                    MpvIpcProperty.EstimatedFrameNumber,
                    cancellationToken)
                .ConfigureAwait(false);
            if (!TryReadEstimatedFrameNumber(previousFrame, out previousFrameNumber))
            {
                return CreateEffectiveFrameObservationFailure(previousFrame);
            }
        }

        var updated = await RunSessionOperationAsync(
            expectedIdentity,
            session => session.UpdateEffects(expectedIdentity!, next),
            cancellationToken).ConfigureAwait(false);
        if (!updated.IsSuccess
            || next is null)
        {
            return updated;
        }

        if (next.Mode is MpvVideoProcessingMode.Original)
        {
            return !observeNextFrame || previousFrameNumber is not ulong originalFrameNumber
                ? updated
                : await WaitForNextFrameAsync(
                        expectedIdentity,
                        originalFrameNumber,
                        cancellationToken)
                    .ConfigureAwait(false);
        }

        var readback = await ReadPropertyAsync(
                expectedIdentity,
                next.Mode is MpvVideoProcessingMode.Cpu4
                    ? MpvIpcProperty.VideoFilterChain
                    : MpvIpcProperty.ShaderOptions,
                cancellationToken)
            .ConfigureAwait(false);
        var readbackResult = next.Mode is MpvVideoProcessingMode.Cpu4
            ? CreateCpu4ReadbackFailure(readback, next) ?? updated
            : CreateGpu83ReadbackFailure(readback, next.ShaderOptions) ?? updated;
        if (!readbackResult.IsSuccess || !observeNextFrame || previousFrameNumber is not ulong frameNumber)
        {
            return readbackResult;
        }

        return await WaitForNextFrameAsync(
                expectedIdentity,
                frameNumber,
                cancellationToken)
            .ConfigureAwait(false);
    }

    /// <summary>
    /// 读取当前 mpv 的固定视频滤镜链属性，供效果提交后的运行时回读使用。
    /// 属性名仍由 <see cref="MpvIpcProperty"/> 白名单约束，不接受任意字符串。
    /// </summary>
    public async Task<MpvIpcDispatchResult> ReadPropertyAsync(
        MediaPlaybackIdentity? expectedIdentity,
        MpvIpcProperty property,
        CancellationToken cancellationToken = default)
    {
        if (expectedIdentity is null || !Enum.IsDefined(property))
        {
            return MpvIpcDispatchResult.Failed(sessionError: new(
                MpvSessionFailureCode.InvalidPlaybackIdentity,
                "mpv 属性读取身份或属性无效。"));
        }

        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return MpvIpcDispatchResult.Failed(sessionError: new(
                MpvSessionFailureCode.InvalidStateTransition,
                "mpv 属性读取已取消。",
                Retryable: true));
        }

        try
        {
            if (!TryGetRunning(expectedIdentity, out var runtime, out _, out var error))
            {
                return MpvIpcDispatchResult.Failed(sessionError: new(
                    MpvSessionFailureCode.SessionClosed,
                    error?.Error?.Message ?? "mpv 会话当前不可用。",
                    Retryable: true));
            }

            return await runtime!.DispatchAsync(
                    MpvIpcCommand.GetProperty(property),
                    expectedIdentity,
                    cancellationToken)
                .ConfigureAwait(false);
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>在当前活动身份下执行绝对 seek。</summary>
    public async Task<WindowsMpvPlaybackControllerResult> SeekAsync(
        MediaPlaybackIdentity? expectedIdentity,
        ulong positionMs,
        CancellationToken cancellationToken = default)
    {
        return await RunSessionOperationAsync(
            expectedIdentity,
            (session) => session.Seek(expectedIdentity!, positionMs),
            cancellationToken).ConfigureAwait(false);
    }

    /// <summary>
    /// 以当前播放身份转发 mpv 状态观察；控制器只转发受管运行时的有限快照，
    /// 不创建后台任务或无界事件队列。
    /// </summary>
    public async IAsyncEnumerable<MpvPlaybackStateMonitorResult> WatchPlaybackStateAsync(
        MediaPlaybackIdentity? expectedIdentity,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        if (expectedIdentity is null)
        {
            yield return MpvPlaybackStateMonitorResult.Failed(new(
                MpvPlaybackStateMonitorFailureCode.InvalidInput,
                "mpv 播放状态监视身份不能为空。"));
            yield break;
        }

        WindowsMpvPlaybackRuntime? runtime;
        lock (_gate)
        {
            runtime = _runtime;
        }

        if (runtime is null)
        {
            yield return MpvPlaybackStateMonitorResult.Failed(new(
                MpvPlaybackStateMonitorFailureCode.RuntimeUnavailable,
                "mpv 播放运行时当前不可用。",
                Retryable: true));
            yield break;
        }

        await foreach (var result in runtime
            .WatchPlaybackStateAsync(expectedIdentity, cancellationToken)
            .ConfigureAwait(false))
        {
            yield return result;
        }
    }

    /// <summary>在同一个 mpv 进程中切换活动源，并保留原播放/暂停意图。</summary>
    public async Task<WindowsMpvPlaybackControllerResult> SwitchSourceAsync(
        SourceMediaDto? source,
        MediaPlaybackIdentity? identity,
        CancellationToken cancellationToken = default)
    {
        if (source is null || identity is null)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.InvalidInput, "播放源或播放身份不能为空。", false);
        }

        if (source.MediaKind is not MediaKind.Video)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.SourceNotVideo, "只有视频源可以绑定 mpv。", false);
        }

        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.Cancelled, "媒体切换已取消。", true);
        }

        try
        {
            if (_activeIdentity is not { } currentIdentity)
            {
                return Failure(WindowsMpvPlaybackControllerFailureCode.NotRunning, "mpv 进程当前未运行。", true);
            }

            if (!TryGetRunning(currentIdentity, out var runtime, out var session, out var error))
            {
                return error!;
            }

            if (!MpvActiveSource.TryCreate(source, identity, out var activeSource, out var sourceError)
                || activeSource is null)
            {
                return Failure(WindowsMpvPlaybackControllerFailureCode.InvalidInput, sourceError?.Message ?? "mpv 活动源无效。", false);
            }

            var wasPlaying = session!.Snapshot.State is MpvSessionState.Playing;
            var previousEffects = session.Snapshot.EffectSnapshot;
            var bound = session.BindSource(activeSource);
            var dispatched = await DispatchCommandsAsync(bound, identity, cancellationToken).ConfigureAwait(false);
            if (!dispatched.IsSuccess)
            {
                await runtime!.StopAsync().ConfigureAwait(false);
                ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                return dispatched;
            }

            var effects = session.UpdateEffects(identity, previousEffects);
            dispatched = await DispatchCommandsAsync(effects, identity, cancellationToken).ConfigureAwait(false);
            if (!dispatched.IsSuccess)
            {
                await runtime!.StopAsync().ConfigureAwait(false);
                ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                return dispatched;
            }

            if (wasPlaying)
            {
                var resumed = session.Start();
                dispatched = await DispatchCommandsAsync(resumed, identity, cancellationToken).ConfigureAwait(false);
                if (!dispatched.IsSuccess)
                {
                    await runtime!.StopAsync().ConfigureAwait(false);
                    ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                    return dispatched;
                }
            }
            else
            {
                // loadfile may clear mpv's previous pause state. Re-assert the paused
                // intent through the session state machine before observing the new source.
                var paused = session.Start();
                dispatched = await DispatchCommandsAsync(paused, identity, cancellationToken).ConfigureAwait(false);
                if (!dispatched.IsSuccess)
                {
                    await runtime!.StopAsync().ConfigureAwait(false);
                    ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                    return dispatched;
                }

                paused = session.Pause();
                dispatched = await DispatchCommandsAsync(paused, identity, cancellationToken).ConfigureAwait(false);
                if (!dispatched.IsSuccess)
                {
                    await runtime!.StopAsync().ConfigureAwait(false);
                    ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                    return dispatched;
                }
            }

            var sourceReady = await WaitForSourceReadyAsync(
                    runtime!,
                    identity,
                    activeSource.MediaPath.CanonicalPath,
                    wasPlaying,
                    cancellationToken)
                .ConfigureAwait(false);
            if (!sourceReady.IsSuccess)
            {
                await runtime!.StopAsync().ConfigureAwait(false);
                ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                return sourceReady;
            }

            lock (_gate)
            {
                _activeIdentity = identity;
                _state = wasPlaying ? WindowsMpvPlaybackControllerState.Playing : WindowsMpvPlaybackControllerState.Paused;
            }

            return Success();
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>
    /// loadfile 的 IPC 成功只代表命令被接收；换源返回前还要确认目标路径、非 EOF
    /// 和播放意图已经出现在 mpv 运行时，播放中额外确认已观察到新源首帧。
    /// </summary>
    private async Task<WindowsMpvPlaybackControllerResult> WaitForSourceReadyAsync(
        WindowsMpvPlaybackRuntime runtime,
        MediaPlaybackIdentity identity,
        string expectedPath,
        bool shouldBePlaying,
        CancellationToken cancellationToken)
    {
        var deadline = DateTime.UtcNow + SourceReadyObservationTimeout;
        while (DateTime.UtcNow < deadline)
        {
            var path = await runtime.DispatchAsync(
                    MpvIpcCommand.GetProperty(MpvIpcProperty.MediaPath),
                    identity,
                    cancellationToken)
                .ConfigureAwait(false);
            if (MpvIpcValueReader.TryReadString(path.Frame!, out var activePath, out _)
                && PathsEqual(activePath, expectedPath))
            {
                var eof = await runtime.DispatchAsync(
                        MpvIpcCommand.GetProperty(MpvIpcProperty.EofReached),
                        identity,
                        cancellationToken)
                    .ConfigureAwait(false);
                var paused = await runtime.DispatchAsync(
                        MpvIpcCommand.GetProperty(MpvIpcProperty.Paused),
                        identity,
                        cancellationToken)
                    .ConfigureAwait(false);
                if (MpvIpcValueReader.TryReadBoolean(eof.Frame!, out var eofReached, out _)
                    && MpvIpcValueReader.TryReadBoolean(paused.Frame!, out var isPaused, out _)
                    && !eofReached
                    && isPaused == !shouldBePlaying)
                {
                    if (!shouldBePlaying)
                    {
                        return Success();
                    }

                    var frame = await runtime.DispatchAsync(
                            MpvIpcCommand.GetProperty(MpvIpcProperty.EstimatedFrameNumber),
                            identity,
                            cancellationToken)
                        .ConfigureAwait(false);
                    if (TryReadEstimatedFrameNumber(frame, out var frameNumber)
                        && frameNumber is > 0)
                    {
                        return Success();
                    }
                }
            }

            try
            {
                await Task.Delay(SourceReadyObservationInterval, cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.Cancelled,
                    "媒体切换观察已取消。",
                    retryable: true);
            }
        }

        return Failure(
            WindowsMpvPlaybackControllerFailureCode.FirstFrameNotObserved,
            "mpv 已接受媒体切换，但在限定时间内未确认新源已加载并恢复播放。",
            retryable: true);
    }

    private static bool PathsEqual(string? currentPath, string expectedPath)
    {
        if (string.IsNullOrWhiteSpace(currentPath))
        {
            return false;
        }

        try
        {
            return StringComparer.OrdinalIgnoreCase.Equals(
                Path.GetFullPath(currentPath),
                Path.GetFullPath(expectedPath));
        }
        catch (ArgumentException)
        {
            return StringComparer.OrdinalIgnoreCase.Equals(currentPath, expectedPath);
        }
    }

    /// <summary>停止并释放 mpv 进程、命名管道和会话。</summary>
    public async Task<WindowsMpvPlaybackControllerResult> ShutdownAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.Cancelled, "mpv 关闭已取消。", true);
        }

        try
        {
            if (_runtime is null)
            {
                lock (_gate)
                {
                    _state = _disposed ? WindowsMpvPlaybackControllerState.Closed : WindowsMpvPlaybackControllerState.Ready;
                }

                return Success();
            }

            var runtime = _runtime;
            var stopped = await runtime.StopAsync(cancellationToken).ConfigureAwait(false);
            await runtime.DisposeAsync().ConfigureAwait(false);
            if (!stopped.IsSuccess)
            {
                ClearRuntime(WindowsMpvPlaybackControllerState.Faulted);
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.StopFailed,
                    stopped.Error?.Message ?? "mpv 播放会话停止失败。",
                    retryable: false);
            }

            ClearRuntime(_disposed ? WindowsMpvPlaybackControllerState.Closed : WindowsMpvPlaybackControllerState.Ready);
            return Success();
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.Cancelled, "mpv 关闭已取消。", true);
        }
        finally
        {
            _serial.Release();
        }
    }

    public async ValueTask DisposeAsync()
    {
        lock (_gate)
        {
            _disposed = true;
        }

        await ShutdownAsync().ConfigureAwait(false);
        GC.SuppressFinalize(this);
    }

    private async Task<WindowsMpvPlaybackControllerResult> RunSessionOperationAsync(
        MediaPlaybackIdentity? expectedIdentity,
        Func<MpvPlaybackSession, MpvSessionOperationResult> operation,
        CancellationToken cancellationToken)
    {
        if (expectedIdentity is null)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.InvalidInput, "播放身份不能为空。", false);
        }

        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.Cancelled, "mpv 操作已取消。", true);
        }

        try
        {
            if (!TryGetRunning(expectedIdentity, out var runtime, out var session, out var error))
            {
                return error!;
            }

            var result = operation(session!);
            var dispatched = await DispatchCommandsAsync(result, expectedIdentity, cancellationToken).ConfigureAwait(false);
            if (!dispatched.IsSuccess)
            {
                return dispatched;
            }

            lock (_gate)
            {
                _state = session!.Snapshot.State switch
                {
                    MpvSessionState.Playing => WindowsMpvPlaybackControllerState.Playing,
                    MpvSessionState.Paused => WindowsMpvPlaybackControllerState.Paused,
                    MpvSessionState.Ready => WindowsMpvPlaybackControllerState.Ready,
                    _ => WindowsMpvPlaybackControllerState.Faulted,
                };
            }

            return Success();
        }
        finally
        {
            _serial.Release();
        }
    }

    private bool IsPlaying(MediaPlaybackIdentity? expectedIdentity)
    {
        lock (_gate)
        {
            return expectedIdentity is not null
                && _activeIdentity == expectedIdentity
                && _session?.Snapshot.State is MpvSessionState.Playing;
        }
    }

    private async Task<WindowsMpvPlaybackControllerResult> WaitForNextFrameAsync(
        MediaPlaybackIdentity? expectedIdentity,
        ulong previousFrameNumber,
        CancellationToken cancellationToken)
    {
        var deadline = DateTime.UtcNow + NextFrameObservationTimeout;
        while (DateTime.UtcNow < deadline)
        {
            var currentFrame = await ReadPropertyAsync(
                    expectedIdentity,
                    MpvIpcProperty.EstimatedFrameNumber,
                    cancellationToken)
                .ConfigureAwait(false);
            if (!TryReadEstimatedFrameNumber(currentFrame, out var currentFrameNumber))
            {
                return CreateEffectiveFrameObservationFailure(currentFrame);
            }

            if (currentFrameNumber > previousFrameNumber)
            {
                return Success();
            }

            try
            {
                await Task.Delay(NextFrameObservationInterval, cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.Cancelled,
                    "视频效果下一帧观察已取消。",
                    retryable: true);
            }
        }

        return Failure(
            WindowsMpvPlaybackControllerFailureCode.EffectiveFrameNotObserved,
            "视频效果已提交，但在限定时间内未观察到下一视频帧。",
            retryable: true);
    }

    private async Task<WindowsMpvPlaybackControllerResult> WaitForFirstFrameAsync(
        WindowsMpvPlaybackRuntime runtime,
        MediaPlaybackIdentity expectedIdentity,
        CancellationToken cancellationToken)
    {
        var deadline = DateTime.UtcNow + NextFrameObservationTimeout;
        while (DateTime.UtcNow < deadline)
        {
            var currentFrame = await runtime.DispatchAsync(
                    MpvIpcCommand.GetProperty(MpvIpcProperty.EstimatedFrameNumber),
                    expectedIdentity,
                    cancellationToken)
                .ConfigureAwait(false);
            if (!TryReadEstimatedFrameNumber(currentFrame, out var currentFrameNumber))
            {
                var message = currentFrame.IsSuccess
                    ? "mpv 未返回有效的首视频帧编号。"
                    : currentFrame.IpcError?.Message
                        ?? currentFrame.SessionError?.Message
                        ?? "mpv 首视频帧观察失败。";
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.FirstFrameNotObserved,
                    message,
                    retryable: true);
            }

            if (currentFrameNumber is > 0)
            {
                return Success();
            }

            try
            {
                await Task.Delay(NextFrameObservationInterval, cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.Cancelled,
                    "mpv 首视频帧观察已取消。",
                    retryable: true);
            }
        }

        return Failure(
            WindowsMpvPlaybackControllerFailureCode.FirstFrameNotObserved,
            "mpv 已启动，但在限定时间内未观察到首视频帧。",
            retryable: true);
    }

    private static bool TryReadEstimatedFrameNumber(
        MpvIpcDispatchResult result,
        out ulong? frameNumber)
    {
        frameNumber = null;
        if (!result.IsSuccess
            || result.Frame is null
            || !MpvIpcValueReader.TryReadFiniteDouble(
                result.Frame,
                out var value,
                out _)
            || value is not double number
            || number < 0
            || number > ulong.MaxValue
            || number != Math.Truncate(number))
        {
            return false;
        }

        frameNumber = (ulong)number;
        return true;
    }

    private WindowsMpvPlaybackControllerResult CreateEffectiveFrameObservationFailure(
        MpvIpcDispatchResult result)
    {
        var message = result.IsSuccess
            ? "mpv 未返回有效的下一视频帧编号。"
            : result.IpcError?.Message
                ?? result.SessionError?.Message
                ?? "mpv 下一视频帧观察失败。";
        return Failure(
            WindowsMpvPlaybackControllerFailureCode.EffectiveFrameNotObserved,
            message,
            retryable: true);
    }

    private static bool TryCreateInitialEffectSnapshot(
        MpvLaunchMode mode,
        MpvVideoEffectSnapshot? requested,
        out MpvVideoEffectSnapshot? snapshot)
    {
        var expectedMode = mode switch
        {
            MpvLaunchMode.Original => MpvVideoProcessingMode.Original,
            MpvLaunchMode.Cpu4 => MpvVideoProcessingMode.Cpu4,
            MpvLaunchMode.Gpu83 => MpvVideoProcessingMode.Gpu83,
            _ => (MpvVideoProcessingMode?)null,
        };
        if (requested is not null)
        {
            snapshot = requested;
            return expectedMode is not null
                && requested.Mode == expectedMode
                && requested.TryValidate(out _);
        }

        if (mode is MpvLaunchMode.Original)
        {
            snapshot = MpvVideoEffectSnapshot.Default;
            return true;
        }

        var processingMode = mode is MpvLaunchMode.Cpu4
            ? MpvVideoProcessingMode.Cpu4
            : MpvVideoProcessingMode.Gpu83;
        if (processingMode is MpvVideoProcessingMode.Gpu83)
        {
            return MpvVideoEffectSnapshot.TryCreateGpu83BaselineColor(
                brightnessPercent: 0,
                contrastPercent: 100,
                saturationPercent: 100,
                hueRotationDegrees: 0,
                out snapshot,
                out _);
        }

        return MpvVideoEffectSnapshot.TryCreate(
            processingMode,
            brightnessPercent: 0,
            contrastPercent: 100,
            saturationPercent: 100,
            hueRotationDegrees: 0,
            shaderOptions: MpvShaderOptionsSnapshot.Empty,
            out snapshot,
            out _);
    }

    private async Task<WindowsMpvPlaybackControllerResult> DispatchCommandsAsync(
        MpvSessionOperationResult operation,
        MediaPlaybackIdentity expectedIdentity,
        CancellationToken cancellationToken)
    {
        if (!operation.IsSuccess)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.InvalidInput, operation.Error?.Message ?? "mpv 播放操作无效。", false);
        }

        var runtime = _runtime;
        if (runtime is null)
        {
            return Failure(WindowsMpvPlaybackControllerFailureCode.NotRunning, "mpv 进程当前未运行。", true);
        }

        foreach (var command in operation.Commands)
        {
            var dispatched = await runtime.DispatchAsync(command, expectedIdentity, cancellationToken).ConfigureAwait(false);
            if (!dispatched.IsSuccess)
            {
                return Failure(
                    WindowsMpvPlaybackControllerFailureCode.DispatchFailed,
                    dispatched.IpcError?.Message ?? dispatched.SessionError?.Message ?? "mpv IPC 命令失败。",
                    dispatched.IpcError?.Retryable ?? dispatched.SessionError?.Retryable ?? true);
            }
        }

        return Success();
    }

    private bool TryGetRunning(
        MediaPlaybackIdentity expectedIdentity,
        out WindowsMpvPlaybackRuntime? runtime,
        out MpvPlaybackSession? session,
        out WindowsMpvPlaybackControllerResult? error)
    {
        runtime = _runtime;
        session = _session;
        error = null;
        if (_disposed)
        {
            error = Failure(WindowsMpvPlaybackControllerFailureCode.Closed, "mpv 播放控制器已关闭。", false);
            return false;
        }

        if (runtime is null || session is null || runtime.Snapshot.State is not WindowsMpvPlaybackRuntimeState.Running)
        {
            error = Failure(WindowsMpvPlaybackControllerFailureCode.NotRunning, "mpv 进程当前未运行。", true);
            return false;
        }

        if (_activeIdentity != expectedIdentity || session.Snapshot.ActiveSource?.Identity != expectedIdentity)
        {
            error = Failure(WindowsMpvPlaybackControllerFailureCode.InvalidInput, "播放身份已过期。", false);
            return false;
        }

        return true;
    }

    private WindowsMpvPlaybackControllerResult? CreateCpu4ReadbackFailure(
        MpvIpcDispatchResult readback,
        MpvVideoEffectSnapshot expected)
    {
        if (!readback.IsSuccess)
        {
            return Failure(
                WindowsMpvPlaybackControllerFailureCode.DispatchFailed,
                readback.IpcError?.Message
                    ?? readback.SessionError?.Message
                    ?? "CPU4 视频滤镜运行时回读失败。",
                readback.IpcError?.Retryable ?? readback.SessionError?.Retryable ?? true);
        }

        if (readback.Frame?.Data is JsonElement data
            && expected.MatchesCpu4Readback(data))
        {
            return null;
        }

        return Failure(
            WindowsMpvPlaybackControllerFailureCode.DispatchFailed,
            "CPU4 视频滤镜未在 mpv 活动滤镜链中确认。",
            retryable: true);
    }

    private WindowsMpvPlaybackControllerResult? CreateGpu83ReadbackFailure(
        MpvIpcDispatchResult readback,
        MpvShaderOptionsSnapshot expected)
    {
        if (!readback.IsSuccess)
        {
            return Failure(
                WindowsMpvPlaybackControllerFailureCode.DispatchFailed,
                readback.IpcError?.Message
                    ?? readback.SessionError?.Message
                    ?? "GPU83 shader 参数运行时回读失败。",
                readback.IpcError?.Retryable ?? readback.SessionError?.Retryable ?? true);
        }

        if (readback.Frame?.Data is JsonElement data
            && expected.MatchesMpvReadback(data))
        {
            return null;
        }

        return Failure(
            WindowsMpvPlaybackControllerFailureCode.DispatchFailed,
            "GPU83 shader 参数未在 mpv 运行时回读中确认。",
            retryable: true);
    }

    private void ClearRuntime(WindowsMpvPlaybackControllerState state)
    {
        lock (_gate)
        {
            _runtime = null;
            _session = null;
            _activeIdentity = null;
            _state = state;
        }
    }

    private WindowsMpvPlaybackControllerResult Success() => new(true, Snapshot);

    private WindowsMpvPlaybackControllerResult Failure(
        WindowsMpvPlaybackControllerFailureCode code,
        string message,
        bool retryable) =>
        new(false, Snapshot, new(code, message, retryable));

    private static WindowsMpvPlaybackRuntimeSnapshot EmptyRuntimeSnapshot() =>
        new(
            WindowsMpvPlaybackRuntimeState.Ready,
            new WindowsMpvHostSnapshot(WindowsMpvHostState.Ready, null, null),
            MpvIpcPipeState.Disconnected,
            null);
}
