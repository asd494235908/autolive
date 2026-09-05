using GpAutoLive.Contracts;

namespace GpAutoLive.Core;

/// <summary>虚拟摄像头状态操作的不可变结果。</summary>
public sealed record VirtualCameraOperationResult(
    bool IsSuccess,
    VirtualCameraStatus Snapshot,
    VirtualCameraError? Error = null)
{
    /// <summary>构造成功结果。</summary>
    public static VirtualCameraOperationResult Success(VirtualCameraStatus snapshot) =>
        new(true, snapshot);

    /// <summary>构造失败结果。</summary>
    public static VirtualCameraOperationResult Failure(VirtualCameraStatus snapshot, VirtualCameraError error) =>
        new(false, snapshot, error);
}

/// <summary>
/// AkVirtualCamera 输出链的纯逻辑所有者。它只管理固定契约、状态、代际、latest-wins 帧和指标，
/// 不创建 WGC/D3D11 资源、不安装设备，也不启动 GPL sidecar。
/// </summary>
public sealed class VirtualCameraOutputManager
{
    private readonly object _gate = new();
    private readonly VirtualCameraConfig _config;
    private readonly Queue<ulong> _readbackSamplesUs = new(VirtualCameraRules.ReadbackSampleCapacity);
    private VirtualCameraState _state = VirtualCameraState.Unavailable;
    private ulong _generation = 1;
    private GpuCaptureFacts? _gpu;
    private uint? _downstreamClientCount;
    private VirtualCameraFrame? _latestFrame;
    private VirtualCameraOutputContext _outputContext;
    private ulong? _lastDeliveredSequence;
    private VirtualCameraMetrics _metrics = new();
    private string? _lastError;

    /// <summary>使用固定默认配置创建纯逻辑输出所有者。</summary>
    public VirtualCameraOutputManager()
    {
        _config = VirtualCameraConfig.Default;
    }

    private VirtualCameraOutputManager(VirtualCameraConfig config)
    {
        _config = config;
    }

    /// <summary>按固定输出合同创建输出所有者，拒绝未支持的配置。</summary>
    public static bool TryCreate(
        VirtualCameraConfig? config,
        out VirtualCameraOutputManager? manager,
        out VirtualCameraError? error)
    {
        manager = null;
        error = null;
        if (config is null || !config.TryValidateFixedOutput(out error))
        {
            error ??= VirtualCameraError.InvalidConfiguration("配置不能为空");
            return false;
        }

        manager = new VirtualCameraOutputManager(config);
        return true;
    }

    /// <summary>获取当前脱敏状态快照。</summary>
    public VirtualCameraStatus Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateStatus();
            }
        }
    }

    /// <summary>获取当前输出上下文。</summary>
    public VirtualCameraOutputContext OutputContext
    {
        get
        {
            lock (_gate)
            {
                return _outputContext;
            }
        }
    }

    /// <summary>同步播放侧事实；输出线程不会自行猜测桌面内容。</summary>
    public void SetOutputContext(VirtualCameraOutputContext context)
    {
        lock (_gate)
        {
            _outputContext = context;
        }
    }

    /// <summary>按当前状态计算应输出黑帧还是最新有效帧。</summary>
    public VirtualCameraOutputPolicy GetOutputPolicy()
    {
        lock (_gate)
        {
            return GetOutputPolicy(_outputContext);
        }
    }

    /// <summary>按指定上下文计算输出策略。</summary>
    public VirtualCameraOutputPolicy GetOutputPolicy(VirtualCameraOutputContext context)
    {
        lock (_gate)
        {
            return IsLiveState()
                && context.PlaybackActive
                && context.VideoSourceActive
                && !context.Paused
                && !context.Stopped
                && !context.Locked
                && context.HasValidFrame
                ? VirtualCameraOutputPolicy.LatestFrame
                : VirtualCameraOutputPolicy.Black;
        }
    }

    /// <summary>记录系统已安装并可被下游枚举的虚拟摄像头组件。</summary>
    public VirtualCameraOperationResult MarkInstalled() => Transition(
        VirtualCameraState.Installed,
        "mark_installed",
        static manager =>
        {
            manager._gpu = null;
            manager._lastError = null;
            return null;
        });

    /// <summary>开始创建 WGC、D3D11 和 sidecar 输出链。</summary>
    public VirtualCameraOperationResult BeginStart() => Transition(
        VirtualCameraState.Starting,
        "begin_start",
        static _ => null);

    /// <summary>提交经过真实性门禁的 GPU 捕获事实并进入 Ready。</summary>
    public VirtualCameraOperationResult MarkReady(GpuCaptureFacts? facts)
    {
        lock (_gate)
        {
            if (_state is not (VirtualCameraState.Starting or VirtualCameraState.Recovering))
            {
                return Failure(VirtualCameraError.InvalidTransition(_state, "mark_ready"));
            }

            VirtualCameraError? error = null;
            if (facts is null || !facts.TryValidateFor(_config, out error))
            {
                _state = VirtualCameraState.Failed;
                _gpu = null;
                _lastError = error?.Message ?? "GPU 捕获事实不能为空";
                InvalidateGeneration();
                return Failure(error ?? VirtualCameraError.GpuGateFailed("GPU 捕获事实不能为空"));
            }

            _gpu = facts;
            _lastError = null;
            _state = VirtualCameraState.Ready;
            return Success();
        }
    }

    /// <summary>标记至少一个下游客户端正在消费。</summary>
    public VirtualCameraOperationResult MarkStreaming() => Transition(
        VirtualCameraState.Streaming,
        "mark_streaming",
        static _ => null);

    /// <summary>
    /// 设置 sidecar 观察到的下游客户端数量。数量为零进入 Ready，正数进入 Streaming；
    /// 未收到观察值时保持 null，不把写入管道误报为正在消费。
    /// </summary>
    public VirtualCameraOperationResult SetDownstreamClientCount(uint count)
    {
        lock (_gate)
        {
            if (!IsLiveState())
            {
                return Failure(VirtualCameraError.InvalidTransition(_state, "set_downstream_client_count"));
            }

            _downstreamClientCount = count;
            _state = count == 0 ? VirtualCameraState.Ready : VirtualCameraState.Streaming;
            return Success();
        }
    }

    /// <summary>开始有界恢复；递增代际并清除旧 GPU 事实和旧帧。</summary>
    public VirtualCameraOperationResult BeginRecovery(string reason)
    {
        lock (_gate)
        {
            if (!IsLiveState())
            {
                return Failure(VirtualCameraError.InvalidTransition(_state, "begin_recovery"));
            }

            InvalidateGeneration();
            _state = VirtualCameraState.Recovering;
            _gpu = null;
            _lastError = NormalizeReason(reason, "虚拟摄像头正在恢复");
            return Success();
        }
    }

    /// <summary>标记确定性失败；失败不会停止本地播放或其他输出。</summary>
    public VirtualCameraOperationResult Fail(string reason)
    {
        lock (_gate)
        {
            InvalidateGeneration();
            _state = VirtualCameraState.Failed;
            _gpu = null;
            _lastError = NormalizeReason(reason, "虚拟摄像头输出失败");
            return Success();
        }
    }

    /// <summary>停止并回收逻辑会话，回到 Installed；Unavailable 是幂等空操作。</summary>
    public VirtualCameraOperationResult Stop()
    {
        lock (_gate)
        {
            if (_state == VirtualCameraState.Unavailable)
            {
                return Success();
            }

            if (_state is not (VirtualCameraState.Starting
                or VirtualCameraState.Ready
                or VirtualCameraState.Streaming
                or VirtualCameraState.Recovering
                or VirtualCameraState.Failed))
            {
                return Failure(VirtualCameraError.InvalidTransition(_state, "stop"));
            }

            _state = VirtualCameraState.Stopping;
            InvalidateGeneration();
            _gpu = null;
            _lastError = null;
            _state = VirtualCameraState.Installed;
            return Success();
        }
    }

    /// <summary>将组件标记为不可用，并使所有旧帧失效。</summary>
    public VirtualCameraOperationResult MarkUnavailable(string reason)
    {
        lock (_gate)
        {
            InvalidateGeneration();
            _state = VirtualCameraState.Unavailable;
            _gpu = null;
            _lastError = NormalizeReason(reason, "虚拟摄像头不可用");
            return Success();
        }
    }

    /// <summary>递增会话代际，使旧 WGC/sidecar 结果不能提交。</summary>
    public ulong AdvanceGeneration()
    {
        lock (_gate)
        {
            InvalidateGeneration();
            return _generation;
        }
    }

    /// <summary>提交一帧；容量为一，已有未消费帧会被最新帧替换并计数。</summary>
    public VirtualCameraOperationResult SubmitFrame(VirtualCameraFrame? frame)
    {
        lock (_gate)
        {
            if (!IsLiveState())
            {
                return Failure(VirtualCameraError.InvalidTransition(_state, "submit_frame"));
            }

            if (frame is null || frame.Payload is null)
            {
                return Failure(VirtualCameraError.InvalidFramePayload());
            }

            if (frame.Generation != _generation)
            {
                _metrics = _metrics with { StaleFramesRejected = SaturatingIncrement(_metrics.StaleFramesRejected) };
                return Failure(VirtualCameraError.StaleGeneration(_generation, frame.Generation));
            }

            if (!_config.TryGetFrameBytes(out var configError, out var expectedBytes))
            {
                return Failure(configError!);
            }

            if (frame.Payload.Length != expectedBytes)
            {
                return Failure(VirtualCameraError.InvalidFrameSize(expectedBytes, frame.Payload.Length));
            }

            _metrics = _metrics with { FramesSubmitted = SaturatingIncrement(_metrics.FramesSubmitted) };
            if (_latestFrame is not null)
            {
                _metrics = _metrics with { FramesDropped = SaturatingIncrement(_metrics.FramesDropped) };
            }

            _latestFrame = frame;
            return Success();
        }
    }

    /// <summary>取出最新帧；取出后才计为已投递，避免虚增下游消费指标。</summary>
    public VirtualCameraFrame? TakeLatestFrame()
    {
        lock (_gate)
        {
            var frame = _latestFrame;
            _latestFrame = null;
            if (frame is null)
            {
                return null;
            }

            _metrics = _metrics with { FramesDelivered = SaturatingIncrement(_metrics.FramesDelivered) };
            if (_lastDeliveredSequence is null || frame.Sequence > _lastDeliveredSequence.Value)
            {
                _lastDeliveredSequence = frame.Sequence;
                _metrics = _metrics with { FrameSequenceAdvances = SaturatingIncrement(_metrics.FrameSequenceAdvances) };
            }

            return frame;
        }
    }

    /// <summary>记录一次 GPU→CPU 回读耗时；只保留最近 512 个样本。</summary>
    public void RecordReadback(TimeSpan elapsed)
    {
        lock (_gate)
        {
            var microseconds = elapsed <= TimeSpan.Zero
                ? 0UL
                : (ulong)Math.Min(elapsed.TotalMicroseconds, ulong.MaxValue);
            if (_readbackSamplesUs.Count == VirtualCameraRules.ReadbackSampleCapacity)
            {
                _readbackSamplesUs.Dequeue();
            }

            _readbackSamplesUs.Enqueue(microseconds);
            _metrics = _metrics with
            {
                ReadbackCount = SaturatingIncrement(_metrics.ReadbackCount)
            };
            RecalculateReadbackPercentiles();
        }
    }

    private VirtualCameraOperationResult Transition(
        VirtualCameraState target,
        string operation,
        Func<VirtualCameraOutputManager, VirtualCameraError?> action)
    {
        lock (_gate)
        {
            var valid = (_state, target) switch
            {
                (VirtualCameraState.Unavailable, VirtualCameraState.Installed) => true,
                (VirtualCameraState.Failed, VirtualCameraState.Installed) => true,
                (VirtualCameraState.Installed, VirtualCameraState.Starting) => true,
                (VirtualCameraState.Ready, VirtualCameraState.Streaming) => true,
                (VirtualCameraState.Recovering, VirtualCameraState.Ready) => true,
                _ => false
            };
            if (!valid)
            {
                return Failure(VirtualCameraError.InvalidTransition(_state, operation));
            }

            var error = action(this);
            if (error is not null)
            {
                return Failure(error);
            }

            _state = target;
            return Success();
        }
    }

    private bool IsLiveState() => _state is VirtualCameraState.Ready or VirtualCameraState.Streaming;

    private VirtualCameraOperationResult Success() => VirtualCameraOperationResult.Success(CreateStatus());

    private VirtualCameraOperationResult Failure(VirtualCameraError error) =>
        VirtualCameraOperationResult.Failure(CreateStatus(), error);

    private VirtualCameraStatus CreateStatus() => new(
        _state,
        _config,
        _generation,
        _gpu,
        _downstreamClientCount,
        _metrics,
        _lastError);

    private void InvalidateGeneration()
    {
        _generation = _generation == ulong.MaxValue ? 1 : _generation + 1;
        if (_latestFrame is not null)
        {
            _metrics = _metrics with { FramesDropped = SaturatingIncrement(_metrics.FramesDropped) };
            _latestFrame = null;
        }

        _downstreamClientCount = null;
        _lastDeliveredSequence = null;
    }

    private void RecalculateReadbackPercentiles()
    {
        var samples = _readbackSamplesUs.ToArray();
        Array.Sort(samples);
        var sum = 0UL;
        foreach (var sample in samples)
        {
            sum = SaturatingAdd(sum, sample);
        }

        _metrics = _metrics with
        {
            ReadbackAverageUs = samples.Length == 0 ? null : sum / (ulong)samples.Length,
            ReadbackP50Us = samples.Length == 0 ? null : samples[PercentileIndex(samples.Length, 0.50)],
            ReadbackP95Us = samples.Length == 0 ? null : samples[PercentileIndex(samples.Length, 0.95)],
            ReadbackP99Us = samples.Length == 0 ? null : samples[PercentileIndex(samples.Length, 0.99)]
        };
    }

    private static int PercentileIndex(int length, double quantile) =>
        Math.Min(length - 1, (int)Math.Ceiling((length - 1) * quantile));

    private static ulong SaturatingIncrement(ulong value) => value == ulong.MaxValue ? value : value + 1;

    private static ulong SaturatingAdd(ulong left, ulong right) =>
        ulong.MaxValue - left < right ? ulong.MaxValue : left + right;

    private static string NormalizeReason(string? reason, string fallback) =>
        string.IsNullOrWhiteSpace(reason) ? fallback : reason.Trim()[..Math.Min(reason.Trim().Length, 512)];
}
