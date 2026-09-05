using System.Diagnostics;
using System.Runtime.InteropServices;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using Vortice.Direct3D;
using Vortice.Direct3D11;
using Vortice.D3DCompiler;
using Vortice.DXGI;
using Vortice.Mathematics;

using VorticeMapFlags = Vortice.Direct3D11.MapFlags;

namespace GpAutoLive.Windows;

/// <summary>GPU→YUY2 转换的稳定结果码。</summary>
public enum WindowsGraphicsCaptureGpuYuy2ConversionCode
{
    Converted,
    Pending,
    NotWindows,
    InvalidFrame,
    SurfaceUnavailable,
    PipelineUnavailable,
    DeviceRemoved,
    ConversionFailed,
    Closed,
}

/// <summary>GPU→YUY2 转换结果；失败时不暴露原生异常正文。</summary>
public sealed record WindowsGraphicsCaptureGpuYuy2ConversionResult(
    bool IsSuccess,
    WindowsGraphicsCaptureGpuYuy2ConversionCode Code,
    VirtualCameraFrame? Frame,
    TimeSpan ReadbackDuration);

/// <summary>
/// 在 WGC 所属的同一硬件 D3D11 设备上完成缩放、BT.601 limited-range 色彩转换和一次有界回读。
/// 输出纹理为 640×720 的 RGBA，每个像素打包两个 YUY2 像素；回读使用三个 staging 槽并丢弃过期帧。
/// </summary>
public sealed class WindowsGraphicsCaptureGpuYuy2Converter : IDisposable
{
    private const int OutputWidth = 1280;
    private const int OutputHeight = 720;
    private const int PackedWidth = OutputWidth / 2;
    private const int StagingCount = 3;
    private static readonly Guid Texture2DIid =
        new("6f15aaf2-d208-4e89-9ab4-489535d34f9c");

    private readonly VirtualCameraConfig _config;
    private readonly Slot[] _slots = new Slot[StagingCount];
    private ID3D11Texture2D? _packedTexture;
    private ID3D11RenderTargetView? _packedRenderTarget;
    private ID3D11VertexShader? _vertexShader;
    private ID3D11PixelShader? _pixelShader;
    private ID3D11SamplerState? _sampler;
    private IntPtr _devicePointer;
    private int _nextSlot;
    private ulong _lastDeliveredSequence;
    private bool _closed;

    public WindowsGraphicsCaptureGpuYuy2Converter(VirtualCameraConfig? config = null)
    {
        _config = config ?? VirtualCameraConfig.Default;
        if (!_config.TryValidateFixedOutput(out var error))
        {
            throw new ArgumentException(error?.Message ?? "虚拟摄像头配置无效", nameof(config));
        }
    }

    /// <summary>同步消费一个 WGC frame；没有完成回读时返回 Pending，不阻塞等待 GPU。</summary>
    public WindowsGraphicsCaptureGpuYuy2ConversionResult TryConvert(
        global::Windows.Graphics.Capture.Direct3D11CaptureFrame frame,
        WindowsGraphicsCaptureD3D11Context context,
        ulong generation,
        ulong sequence)
    {
        ArgumentNullException.ThrowIfNull(frame);
        ArgumentNullException.ThrowIfNull(context);
        if (_closed)
        {
            return Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.Closed);
        }

        if (!OperatingSystem.IsWindows())
        {
            return Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.NotWindows);
        }

        if (!TryEnsurePipeline(context, out var pipelineCode))
        {
            return Failure(pipelineCode);
        }

        // 先回收已完成的旧槽位，容量保持为三，避免 GPU 队列无界增长。
        var latest = PollCompleted(context.ImmediateContext);
        if (!TryGetFreeSlot(out var slotIndex))
        {
            return latest ?? Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.Pending);
        }

        if (!TryGetTexture(frame, out var sourceTexture))
        {
            return latest ?? Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.SurfaceUnavailable);
        }

        try
        {
            return TryConvertTexture(
                sourceTexture!,
                context,
                generation,
                sequence,
                To90Khz(frame.SystemRelativeTime.Ticks),
                slotIndex,
                latest);
        }
        catch (SharpGen.Runtime.SharpGenException)
        {
            return latest ?? Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.ConversionFailed);
        }
        catch (COMException)
        {
            return latest ?? Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.DeviceRemoved);
        }
        finally
        {
            sourceTexture!.Dispose();
        }
    }

    /// <summary>
    /// 在同一 D3D11 设备上转换一个 BGRA 纹理；用于真实 WGC 回调和 Windows GPU 冒烟夹具。
    /// 调用方保留 sourceTexture 的所有权，本方法不会释放它。
    /// </summary>
    public WindowsGraphicsCaptureGpuYuy2ConversionResult TryConvertTexture(
        ID3D11Texture2D sourceTexture,
        WindowsGraphicsCaptureD3D11Context context,
        ulong generation,
        ulong sequence,
        ulong timestamp90Khz)
    {
        ArgumentNullException.ThrowIfNull(sourceTexture);
        ArgumentNullException.ThrowIfNull(context);
        if (_closed)
        {
            return Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.Closed);
        }

        if (!TryEnsurePipeline(context, out var pipelineCode))
        {
            return Failure(pipelineCode);
        }

        var latest = PollCompleted(context.ImmediateContext);
        if (!TryGetFreeSlot(out var slotIndex))
        {
            return latest ?? Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.Pending);
        }

        return TryConvertTexture(sourceTexture, context, generation, sequence, timestamp90Khz, slotIndex, latest);
    }

    /// <summary>轮询三槽 staging 回读；不提交新 GPU 工作，适合下游写入背压时使用。</summary>
    public WindowsGraphicsCaptureGpuYuy2ConversionResult TryDrain(
        WindowsGraphicsCaptureD3D11Context context)
    {
        ArgumentNullException.ThrowIfNull(context);
        if (_closed)
        {
            return Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.Closed);
        }

        return PollCompleted(context.ImmediateContext)
            ?? SuccessPending();
    }

    private WindowsGraphicsCaptureGpuYuy2ConversionResult TryConvertTexture(
        ID3D11Texture2D sourceTexture,
        WindowsGraphicsCaptureD3D11Context context,
        ulong generation,
        ulong sequence,
        ulong timestamp90Khz,
        int slotIndex,
        WindowsGraphicsCaptureGpuYuy2ConversionResult? latest)
    {
        try
        {
            using var sourceView = context.Device.CreateShaderResourceView(sourceTexture, null);
            var immediateContext = context.ImmediateContext;
            immediateContext.OMSetRenderTargets(_packedRenderTarget!, null);
            immediateContext.RSSetViewports(new[] { new Viewport(0, 0, PackedWidth, OutputHeight) });
            immediateContext.IASetPrimitiveTopology(PrimitiveTopology.TriangleList);
            immediateContext.VSSetShader(_vertexShader!);
            immediateContext.PSSetShader(_pixelShader!);
            immediateContext.PSSetShaderResources(0, new[] { sourceView });
            immediateContext.PSSetSamplers(0, new[] { _sampler! });
            immediateContext.Draw(3, 0);
            immediateContext.CopyResource(_slots[slotIndex].Staging, _packedTexture!);
            immediateContext.Flush();
            _slots[slotIndex] = _slots[slotIndex] with
            {
                Pending = true,
                Sequence = sequence,
                Generation = generation,
                Timestamp90Khz = timestamp90Khz,
            };
            _nextSlot = (slotIndex + 1) % StagingCount;
        }
        catch (SharpGen.Runtime.SharpGenException)
        {
            return latest ?? Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.ConversionFailed);
        }
        catch (COMException)
        {
            return latest ?? Failure(WindowsGraphicsCaptureGpuYuy2ConversionCode.DeviceRemoved);
        }

        var completed = PollCompleted(context.ImmediateContext);
        return completed ?? latest ?? SuccessPending();
    }

    public void Dispose()
    {
        if (_closed)
        {
            return;
        }

        _closed = true;
        foreach (var slot in _slots)
        {
            slot.Staging?.Dispose();
        }

        _sampler?.Dispose();
        _pixelShader?.Dispose();
        _vertexShader?.Dispose();
        _packedRenderTarget?.Dispose();
        _packedTexture?.Dispose();
        _sampler = null;
        _pixelShader = null;
        _vertexShader = null;
        _packedRenderTarget = null;
        _packedTexture = null;
        _devicePointer = IntPtr.Zero;
        _lastDeliveredSequence = 0;
        GC.SuppressFinalize(this);
    }

    private bool TryEnsurePipeline(
        WindowsGraphicsCaptureD3D11Context context,
        out WindowsGraphicsCaptureGpuYuy2ConversionCode code)
    {
        code = WindowsGraphicsCaptureGpuYuy2ConversionCode.PipelineUnavailable;
        var device = context.Device;
        if (_devicePointer == device.NativePointer && _packedTexture is not null)
        {
            code = WindowsGraphicsCaptureGpuYuy2ConversionCode.Converted;
            return true;
        }

        DisposePipelineResources();
        try
        {
            var vertexCode = CompileShader(VertexShaderSource, "VSMain", "vs_5_0");
            var pixelCode = CompileShader(PixelShaderSource, "PSMain", "ps_5_0");
            _vertexShader = device.CreateVertexShader(vertexCode, null);
            _pixelShader = device.CreatePixelShader(pixelCode, null);

            _sampler = device.CreateSamplerState(new SamplerDescription(
                Filter.MinMagMipPoint,
                TextureAddressMode.Clamp,
                TextureAddressMode.Clamp,
                TextureAddressMode.Clamp,
                0,
                1,
                ComparisonFunction.Never,
                0,
                float.MaxValue));

            var packedDescription = new Texture2DDescription(
                Format.R8G8B8A8_UNorm,
                PackedWidth,
                OutputHeight,
                1,
                1,
                BindFlags.RenderTarget,
                ResourceUsage.Default,
                CpuAccessFlags.None,
                1,
                0,
                ResourceOptionFlags.None);
            _packedTexture = device.CreateTexture2D(in packedDescription);
            _packedRenderTarget = device.CreateRenderTargetView(
                _packedTexture,
                new RenderTargetViewDescription(
                    RenderTargetViewDimension.Texture2D,
                    Format.R8G8B8A8_UNorm,
                    0,
                    0,
                    1));

            var stagingDescription = new Texture2DDescription(
                Format.R8G8B8A8_UNorm,
                PackedWidth,
                OutputHeight,
                1,
                1,
                BindFlags.None,
                ResourceUsage.Staging,
                CpuAccessFlags.Read,
                1,
                0,
                ResourceOptionFlags.None);
            for (var index = 0; index < StagingCount; index++)
            {
                _slots[index] = new(device.CreateTexture2D(in stagingDescription));
            }

            _devicePointer = device.NativePointer;
            code = WindowsGraphicsCaptureGpuYuy2ConversionCode.Converted;
            return true;
        }
        catch (SharpGen.Runtime.SharpGenException)
        {
            DisposePipelineResources();
            return false;
        }
        catch (COMException)
        {
            DisposePipelineResources();
            return false;
        }
    }

    private void DisposePipelineResources()
    {
        foreach (var slot in _slots)
        {
            slot.Staging?.Dispose();
        }

        Array.Clear(_slots);
        _sampler?.Dispose();
        _pixelShader?.Dispose();
        _vertexShader?.Dispose();
        _packedRenderTarget?.Dispose();
        _packedTexture?.Dispose();
        _sampler = null;
        _pixelShader = null;
        _vertexShader = null;
        _packedRenderTarget = null;
        _packedTexture = null;
        _devicePointer = IntPtr.Zero;
        _nextSlot = 0;
        _lastDeliveredSequence = 0;
    }

    private WindowsGraphicsCaptureGpuYuy2ConversionResult? PollCompleted(ID3D11DeviceContext context)
    {
        WindowsGraphicsCaptureGpuYuy2ConversionResult? latest = null;
        for (var index = 0; index < StagingCount; index++)
        {
            var slot = _slots[index];
            if (!slot.Pending || slot.Staging is null)
            {
                continue;
            }

            var stopwatch = Stopwatch.StartNew();
            var mapResult = context.Map(
                slot.Staging,
                0,
                MapMode.Read,
                VorticeMapFlags.DoNotWait,
                out var mapped);
            if (!mapResult.Success)
            {
                continue;
            }

            try
            {
                var payload = new byte[VirtualCameraRules.Width * VirtualCameraRules.Height * 2];
                var rowBytes = PackedWidth * 4;
                for (var row = 0; row < OutputHeight; row++)
                {
                    Marshal.Copy(
                        IntPtr.Add(mapped.DataPointer, checked((int)(row * mapped.RowPitch))),
                        payload,
                        row * rowBytes,
                        rowBytes);
                }

                stopwatch.Stop();
                if (slot.Sequence > _lastDeliveredSequence
                    && (latest?.Frame is not VirtualCameraFrame current
                        || slot.Sequence > current.Sequence))
                {
                    latest = new(
                        true,
                        WindowsGraphicsCaptureGpuYuy2ConversionCode.Converted,
                        new VirtualCameraFrame(slot.Generation, slot.Sequence, slot.Timestamp90Khz, payload),
                        stopwatch.Elapsed);
                }
            }
            finally
            {
                context.Unmap(slot.Staging, 0);
                _slots[index] = slot with { Pending = false };
            }
        }

        if (latest?.Frame is VirtualCameraFrame delivered)
        {
            _lastDeliveredSequence = delivered.Sequence;
        }

        return latest;
    }

    private bool TryGetFreeSlot(out int index)
    {
        for (var offset = 0; offset < StagingCount; offset++)
        {
            var candidate = (_nextSlot + offset) % StagingCount;
            if (!_slots[candidate].Pending)
            {
                index = candidate;
                return true;
            }
        }

        index = -1;
        return false;
    }

    private static bool TryGetTexture(
        global::Windows.Graphics.Capture.Direct3D11CaptureFrame frame,
        out ID3D11Texture2D? texture)
    {
        texture = null;
        try
        {
            if (frame.Surface is not WinRT.IWinRTObject winRtSurface)
            {
                return false;
            }

            // WinRT projection 对 IDirect3DSurface 保留了原生 IObjectReference；
            // 直接对 Marshal.GetIUnknownForObject 的结果做二次 RCW 转换会在
            // .NET 10/WGC 帧上丢失 IDirect3DDxgiInterfaceAccess。通过同一个
            // WinRT object reference 走 Rust 对应的 surface.cast() ABI。
            var access = winRtSurface.NativeObject.AsInterface<IDirect3DDxgiInterfaceAccess>();
            try
            {
                var textureIid = Texture2DIid;
                if (access.GetInterface(in textureIid, out var texturePointer) < 0
                    || texturePointer == IntPtr.Zero)
                {
                    return false;
                }

                try
                {
                    texture = new ID3D11Texture2D(texturePointer);
                    return true;
                }
                catch (Exception)
                {
                    Marshal.Release(texturePointer);
                    return false;
                }
            }
            finally
            {
                Marshal.ReleaseComObject(access);
            }
        }
        catch (COMException)
        {
            return false;
        }
        catch (InvalidCastException)
        {
            return false;
        }
    }

    private static byte[] CompileShader(string source, string entryPoint, string target)
    {
        var bytecode = Compiler.Compile(
            source,
            entryPoint,
            "GpAutoLive.WindowsGraphicsCaptureGpuYuy2Converter",
            target,
            ShaderFlags.OptimizationLevel3,
            EffectFlags.None);
        return bytecode.ToArray();
    }

    private static ulong To90Khz(long ticks)
    {
        if (ticks <= 0)
        {
            return 0;
        }

        var seconds = ticks / TimeSpan.TicksPerSecond;
        var remainder = ticks % TimeSpan.TicksPerSecond;
        return checked((ulong)(seconds * 90_000 + remainder * 90_000 / TimeSpan.TicksPerSecond));
    }

    private static WindowsGraphicsCaptureGpuYuy2ConversionResult Failure(
        WindowsGraphicsCaptureGpuYuy2ConversionCode code) =>
        new(false, code, null, TimeSpan.Zero);

    private static WindowsGraphicsCaptureGpuYuy2ConversionResult SuccessPending() =>
        new(true, WindowsGraphicsCaptureGpuYuy2ConversionCode.Pending, null, TimeSpan.Zero);

    private readonly record struct Slot(
        ID3D11Texture2D? Staging,
        bool Pending = false,
        ulong Generation = 0,
        ulong Sequence = 0,
        ulong Timestamp90Khz = 0);

    [ComImport]
    [Guid("a9b3d012-3df2-4ee3-b8d1-8695f457d3c1")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IDirect3DDxgiInterfaceAccess
    {
        [PreserveSig]
        int GetInterface(in Guid iid, out IntPtr objectPointer);
    }

    private const string VertexShaderSource = """
        struct VSOut { float4 position : SV_Position; float2 uv : TEXCOORD0; };
        VSOut VSMain(uint vertexId : SV_VertexID) {
            float2 position = vertexId == 2 ? float2(3.0, -1.0) : (vertexId == 1 ? float2(-1.0, 3.0) : float2(-1.0, -1.0));
            VSOut output;
            output.position = float4(position, 0.0, 1.0);
            output.uv = float2((position.x + 1.0) * 0.5, 1.0 - (position.y + 1.0) * 0.5);
            return output;
        }
        """;

    private const string PixelShaderSource = """
        Texture2D sourceTexture : register(t0);
        SamplerState pointSampler : register(s0);
        float3 ToLimitedYuv(float3 rgb) {
            float y = 0.257 * rgb.r + 0.504 * rgb.g + 0.098 * rgb.b + 0.0625;
            float u = -0.148 * rgb.r - 0.291 * rgb.g + 0.439 * rgb.b + 0.5;
            float v = 0.439 * rgb.r - 0.368 * rgb.g - 0.071 * rgb.b + 0.5;
            return saturate(float3(y, u, v));
        }
        struct VSOut { float4 position : SV_Position; float2 uv : TEXCOORD0; };
        float4 PSMain(VSOut input) : SV_Target {
            uint sourceWidth, sourceHeight;
            sourceTexture.GetDimensions(sourceWidth, sourceHeight);
            float2 sourcePixel = float2(1.0 / sourceWidth, 1.0 / sourceHeight);
            float3 yuv0 = ToLimitedYuv(sourceTexture.Sample(
                pointSampler,
                input.uv - float2(sourcePixel.x * 0.5, 0.0)));
            float3 yuv1 = ToLimitedYuv(sourceTexture.Sample(
                pointSampler,
                input.uv + float2(sourcePixel.x * 0.5, 0.0)));
            return float4(yuv0.x, (yuv0.y + yuv1.y) * 0.5, yuv1.x, (yuv0.z + yuv1.z) * 0.5);
        }
        """;
}
