using System.Diagnostics.CodeAnalysis;
using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>AkVirtualCamera 首版唯一允许的像素格式。</summary>
public enum VirtualCameraPixelFormat
{
    /// <summary>YUY2 4:2:2 packed 格式。</summary>
    [JsonStringEnumMemberName("YUY2")]
    Yuy2
}

/// <summary>虚拟摄像头固定输出配置；不包含设备安装或系统权限操作。</summary>
public sealed record VirtualCameraConfig
{
    /// <summary>下游应用看到的固定设备名称。</summary>
    [property: JsonPropertyName("device_name")]
    public string DeviceName { get; init; } = VirtualCameraRules.DeviceName;

    /// <summary>输出像素格式。</summary>
    [property: JsonPropertyName("pixel_format")]
    public VirtualCameraPixelFormat PixelFormat { get; init; } = VirtualCameraPixelFormat.Yuy2;

    /// <summary>输出宽度。</summary>
    [property: JsonPropertyName("width")]
    public uint Width { get; init; } = VirtualCameraRules.Width;

    /// <summary>输出高度。</summary>
    [property: JsonPropertyName("height")]
    public uint Height { get; init; } = VirtualCameraRules.Height;

    /// <summary>输出帧率。</summary>
    [property: JsonPropertyName("fps")]
    public uint Fps { get; init; } = VirtualCameraRules.Fps;

    /// <summary>首版固定为 false，表示末端允许一次有界 GPU→CPU 回读。</summary>
    [property: JsonPropertyName("zero_copy")]
    public bool ZeroCopy { get; init; }

    /// <summary>计算单帧字节数，并拒绝溢出或过大值。</summary>
    public bool TryGetFrameBytes([NotNullWhen(false)] out VirtualCameraError? error, out int frameBytes)
    {
        error = null;
        frameBytes = 0;
        if (Width == 0 || Height == 0)
        {
            error = VirtualCameraError.InvalidConfiguration("输出尺寸不能为空");
            return false;
        }

        try
        {
            var bytes = checked((ulong)Width * Height * 2);
            if (bytes > VirtualCameraRules.MaxFrameBytes || bytes > int.MaxValue)
            {
                error = VirtualCameraError.InvalidConfiguration("输出帧大小超出受控范围");
                return false;
            }

            frameBytes = (int)bytes;
            return true;
        }
        catch (OverflowException)
        {
            error = VirtualCameraError.InvalidConfiguration("输出帧大小计算溢出");
            return false;
        }
    }

    /// <summary>校验首版固定 YUY2 1280×720@30fps 合同。</summary>
    public bool TryValidateFixedOutput([NotNullWhen(false)] out VirtualCameraError? error)
    {
        error = null;
        if (!string.Equals(DeviceName, VirtualCameraRules.DeviceName, StringComparison.Ordinal)
            || PixelFormat != VirtualCameraPixelFormat.Yuy2
            || Width != VirtualCameraRules.Width
            || Height != VirtualCameraRules.Height
            || Fps != VirtualCameraRules.Fps
            || ZeroCopy)
        {
            error = VirtualCameraError.InvalidConfiguration(
                "首版虚拟摄像头只允许 YUY2 1280×720@30fps 且 zero_copy=false");
            return false;
        }

        return TryGetFrameBytes(out error, out _);
    }

    /// <summary>首版虚拟摄像头默认配置。</summary>
    public static VirtualCameraConfig Default { get; } = new();
}

/// <summary>虚拟摄像头固定常量和内存边界。</summary>
public static class VirtualCameraRules
{
    /// <summary>固定设备名称。</summary>
    public const string DeviceName = "GpAutoLive Camera";
    /// <summary>固定输出宽度。</summary>
    public const uint Width = 1280;
    /// <summary>固定输出高度。</summary>
    public const uint Height = 720;
    /// <summary>固定输出帧率。</summary>
    public const uint Fps = 30;
    /// <summary>固定捕获 API 名称。</summary>
    public const string CaptureApi = "windows_graphics_capture";
    /// <summary>固定 AkVirtualCamera 传输标识。</summary>
    public const string Transport = "akvcam_mmap_cpu";
    /// <summary>单帧最大字节数。</summary>
    public const ulong MaxFrameBytes = 64UL * 1024 * 1024;
    /// <summary>回读指标有界样本容量。</summary>
    public const int ReadbackSampleCapacity = 512;
}

/// <summary>虚拟摄像头运行状态。</summary>
public enum VirtualCameraState
{
    /// <summary>组件未安装或系统不满足门禁。</summary>
    Unavailable,
    /// <summary>组件已安装但输出链未启动。</summary>
    Installed,
    /// <summary>正在创建 WGC、D3D11 和 sidecar 链。</summary>
    Starting,
    /// <summary>输出链已就绪且尚无下游客户端。</summary>
    Ready,
    /// <summary>至少有一个下游客户端正在消费。</summary>
    Streaming,
    /// <summary>窗口、GPU 或 sidecar 正在有界重建。</summary>
    Recovering,
    /// <summary>确定性错误或重建预算耗尽。</summary>
    Failed,
    /// <summary>正在停止并回收资源。</summary>
    Stopping
}

/// <summary>Windows WGC/D3D11 捕获真实性事实。</summary>
public sealed record GpuCaptureFacts(
    string CaptureApi,
    string AdapterLuid,
    string AdapterName,
    uint VendorId,
    uint DeviceId,
    string FeatureLevel,
    bool IsWarp,
    bool GpuScale,
    bool GpuColorConvert,
    string Transport,
    bool ZeroCopy,
    uint Width,
    uint Height,
    uint Fps)
{
    /// <summary>验证事实是否满足固定虚拟摄像头输出合同。</summary>
    public bool TryValidateFor(VirtualCameraConfig config, [NotNullWhen(false)] out VirtualCameraError? error)
    {
        error = null;
        if (!string.Equals(CaptureApi, VirtualCameraRules.CaptureApi, StringComparison.Ordinal))
        {
            error = VirtualCameraError.GpuGateFailed("捕获 API 不是 Windows Graphics Capture");
        }
        else if (string.IsNullOrWhiteSpace(AdapterLuid))
        {
            error = VirtualCameraError.GpuGateFailed("D3D11 adapter LUID 缺失");
        }
        else if (VendorId == 0)
        {
            error = VirtualCameraError.GpuGateFailed("D3D11 adapter 厂商 ID 缺失");
        }
        else if (string.IsNullOrWhiteSpace(AdapterName) || string.IsNullOrWhiteSpace(FeatureLevel))
        {
            error = VirtualCameraError.GpuGateFailed("GPU 设备名称或 feature level 缺失");
        }
        else if (IsWarp)
        {
            error = VirtualCameraError.GpuGateFailed("WARP 软件适配器不能作为 GPU 输出");
        }
        else if (!GpuScale || !GpuColorConvert)
        {
            error = VirtualCameraError.GpuGateFailed("缩放和色彩转换必须由 GPU 完成");
        }
        else if (!string.Equals(Transport, VirtualCameraRules.Transport, StringComparison.Ordinal) || ZeroCopy)
        {
            error = VirtualCameraError.GpuGateFailed("首版只允许 AkVirtualCamera CPU raw 传输且 zero_copy=false");
        }
        else if (Width != config.Width || Height != config.Height || Fps != config.Fps)
        {
            error = VirtualCameraError.GpuGateFailed("实际输出规格与固定 720p30 合同不一致");
        }

        return error is null;
    }
}

/// <summary>虚拟摄像头输出上下文；由播放所有者显式同步。</summary>
public readonly record struct VirtualCameraOutputContext(
    bool PlaybackActive,
    bool VideoSourceActive,
    bool Paused,
    bool Stopped,
    bool Locked,
    bool HasValidFrame);

/// <summary>无有效最终画面时的输出策略。</summary>
public enum VirtualCameraOutputPolicy
{
    /// <summary>输出 YUY2 limited-range 黑帧。</summary>
    Black,
    /// <summary>输出最新有效帧。</summary>
    LatestFrame
}

/// <summary>虚拟摄像头帧；payload 必须是固定大小的 YUY2 数据。</summary>
public sealed record VirtualCameraFrame(
    ulong Generation,
    ulong Sequence,
    ulong Timestamp90Khz,
    byte[] Payload)
{
    /// <summary>创建 YUY2 limited-range 黑帧，避免全零造成绿色偏色。</summary>
    public static bool TryCreateBlack(
        VirtualCameraConfig config,
        ulong generation,
        ulong sequence,
        ulong timestamp90Khz,
        [NotNullWhen(true)] out VirtualCameraFrame? frame,
        [NotNullWhen(false)] out VirtualCameraError? error)
    {
        frame = null;
        if (!config.TryValidateFixedOutput(out error)
            || !config.TryGetFrameBytes(out error, out var frameBytes))
        {
            return false;
        }

        var payload = new byte[frameBytes];
        for (var index = 0; index < payload.Length; index += 4)
        {
            payload[index] = 16;
            payload[index + 1] = 128;
            payload[index + 2] = 16;
            payload[index + 3] = 128;
        }

        frame = new VirtualCameraFrame(generation, sequence, timestamp90Khz, payload);
        return true;
    }
}

/// <summary>虚拟摄像头运行指标；所有计数器均饱和递增。</summary>
public sealed record VirtualCameraMetrics(
    ulong FramesSubmitted = 0,
    ulong FramesDelivered = 0,
    ulong FramesDropped = 0,
    ulong StaleFramesRejected = 0,
    ulong FrameSequenceAdvances = 0,
    ulong ReadbackCount = 0,
    ulong? ReadbackAverageUs = null,
    ulong? ReadbackP50Us = null,
    ulong? ReadbackP95Us = null,
    ulong? ReadbackP99Us = null);

/// <summary>虚拟摄像头脱敏运行状态。</summary>
public sealed record VirtualCameraStatus(
    VirtualCameraState State,
    VirtualCameraConfig Config,
    ulong Generation,
    GpuCaptureFacts? Gpu,
    uint? DownstreamClientCount,
    VirtualCameraMetrics Metrics,
    string? LastError);

/// <summary>虚拟摄像头状态边界的稳定错误。</summary>
public sealed record VirtualCameraError(string Code, string Message)
{
    /// <summary>构造无效配置错误。</summary>
    public static VirtualCameraError InvalidConfiguration(string message) =>
        new("virtual_camera_config_invalid", $"无效的虚拟摄像头配置：{message}");

    /// <summary>构造非法状态转换错误。</summary>
    public static VirtualCameraError InvalidTransition(VirtualCameraState state, string operation) =>
        new("virtual_camera_invalid_transition", $"虚拟摄像头状态 {state} 不允许执行 {operation}");

    /// <summary>构造 GPU 准入失败错误。</summary>
    public static VirtualCameraError GpuGateFailed(string message) =>
        new("virtual_camera_gpu_gate_failed", $"GPU 虚拟摄像头准入失败：{message}");

    /// <summary>构造过期代际错误。</summary>
    public static VirtualCameraError StaleGeneration(ulong expected, ulong received) =>
        new("virtual_camera_stale_generation", $"虚拟摄像头帧代际过期：当前 {expected}，收到 {received}");

    /// <summary>构造帧大小错误。</summary>
    public static VirtualCameraError InvalidFrameSize(int expected, int received) =>
        new("virtual_camera_frame_size_invalid", $"虚拟摄像头帧大小无效：需要 {expected} 字节，收到 {received} 字节");

    /// <summary>构造缺失帧负载错误。</summary>
    public static VirtualCameraError InvalidFramePayload() =>
        new("virtual_camera_frame_payload_missing", "虚拟摄像头帧负载不能为空");
}
