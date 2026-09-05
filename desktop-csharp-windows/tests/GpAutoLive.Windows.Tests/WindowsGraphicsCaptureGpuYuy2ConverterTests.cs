using Vortice.Direct3D11;
using Vortice.DXGI;
using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
[DoNotParallelize]
public sealed class WindowsGraphicsCaptureGpuYuy2ConverterTests
{
    [TestMethod]
    public void HardwareDevice_RejectsWarpAndReportsStableResult()
    {
        var result = WindowsD3D11HardwareContextFactory.TryCreate();
        if (!OperatingSystem.IsWindows())
        {
            Assert.AreEqual(WindowsD3D11HardwareContextCode.NotWindows, result.Code);
            return;
        }

        if (result.IsSuccess)
        {
            Assert.AreEqual(WindowsD3D11HardwareContextCode.Ready, result.Code);
            Assert.IsNotNull(result.FeatureLevel);
            result.Context!.Dispose();
        }
        else
        {
            Assert.AreNotEqual(WindowsD3D11HardwareContextCode.Ready, result.Code);
        }
    }

    [TestMethod]
    public void ConstantBgraTexture_IsConvertedOnGpuToLimitedRangeYuy2()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        var deviceResult = WindowsD3D11HardwareContextFactory.TryCreate();
        if (!deviceResult.IsSuccess || deviceResult.Context is null)
        {
            Assert.Inconclusive($"本机没有可用硬件 D3D11：{deviceResult.Code}");
            return;
        }

        using var context = deviceResult.Context;
        using var converter = new WindowsGraphicsCaptureGpuYuy2Converter();
        var sourceDescription = new Texture2DDescription(
            Format.B8G8R8A8_UNorm,
            1280,
            720,
            1,
            1,
            BindFlags.ShaderResource,
            ResourceUsage.Default,
            CpuAccessFlags.None,
            1,
            0,
            ResourceOptionFlags.None);
        using var source = context.Device.CreateTexture2D(in sourceDescription);
        var bgra = new byte[1280 * 720 * 4];
        for (var offset = 0; offset < bgra.Length; offset += 4)
        {
            bgra[offset] = 0;
            bgra[offset + 1] = 0;
            bgra[offset + 2] = 255;
            bgra[offset + 3] = 255;
        }

        context.ImmediateContext.UpdateSubresource(
            bgra,
            source,
            0,
            1280 * 4,
            (uint)bgra.Length,
            null);

        WindowsGraphicsCaptureGpuYuy2ConversionResult? converted = null;
        var lastCode = WindowsGraphicsCaptureGpuYuy2ConversionCode.Pending;
        for (var attempt = 0; attempt < 3; attempt++)
        {
            var result = converter.TryConvertTexture(source, context, 7, (ulong)attempt + 1, 90_000);
            lastCode = result.Code;
            if (result.Frame is not null)
            {
                converted = result;
                break;
            }
        }

        for (var attempt = 0; attempt < 120 && converted?.Frame is null; attempt++)
        {
            var result = converter.TryDrain(context);
            lastCode = result.Code;
            if (result.Frame is not null)
            {
                converted = result;
                break;
            }

            Thread.Sleep(1);
        }

        Assert.IsNotNull(converted?.Frame, $"三槽 GPU 回读应在有界尝试内产出一帧，最后状态 {lastCode}");
        Assert.AreEqual(1280 * 720 * 2, converted!.Frame!.Payload.Length);
        Assert.AreEqual(7UL, converted.Frame.Generation);
        Assert.IsTrue(converted.ReadbackDuration >= TimeSpan.Zero);
        Assert.IsTrue(converted.Frame.Payload[0] is >= 70 and <= 95, "红色输入的 Y 应在 limited-range 红色范围");
        Assert.IsTrue(converted.Frame.Payload[1] is >= 70 and <= 115, "红色输入的 U 应为有限范围蓝差分");
        Assert.IsTrue(converted.Frame.Payload[3] is >= 190 and <= 255, "红色输入的 V 应为有限范围红差分");
        Assert.IsTrue(
            converted.Frame.Payload[^4] is >= 70 and <= 95,
            "全屏三角形必须覆盖输出最后一行的最后一个 YUY2 对；否则最终输出会出现未绘制区域");
        Assert.IsTrue(
            converted.Frame.Payload[^1] is >= 190 and <= 255,
            "全屏三角形必须覆盖输出右下角的 V；否则最终输出画面会出现黑块");
    }

    [TestMethod]
    public void SourceTopLeftPixel_IsNotSkippedByGpuSampling()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        var deviceResult = WindowsD3D11HardwareContextFactory.TryCreate();
        if (!deviceResult.IsSuccess || deviceResult.Context is null)
        {
            Assert.Inconclusive($"本机没有可用硬件 D3D11：{deviceResult.Code}");
            return;
        }

        using var context = deviceResult.Context;
        using var converter = new WindowsGraphicsCaptureGpuYuy2Converter();
        var sourceDescription = new Texture2DDescription(
            Format.B8G8R8A8_UNorm,
            1280,
            720,
            1,
            1,
            BindFlags.ShaderResource,
            ResourceUsage.Default,
            CpuAccessFlags.None,
            1,
            0,
            ResourceOptionFlags.None);
        using var source = context.Device.CreateTexture2D(in sourceDescription);
        var bgra = new byte[1280 * 720 * 4];
        for (var offset = 3; offset < bgra.Length; offset += 4)
        {
            bgra[offset] = 255;
        }

        // 只点亮源纹理左上角；C# shader 若沿用旧的半像素偏移，输出会采到黑色的下一行/列。
        bgra[2] = 255;
        context.ImmediateContext.UpdateSubresource(
            bgra,
            source,
            0,
            1280 * 4,
            (uint)bgra.Length,
            null);

        WindowsGraphicsCaptureGpuYuy2ConversionResult? converted = null;
        for (var attempt = 0; attempt < 3; attempt++)
        {
            var result = converter.TryConvertTexture(source, context, 11, (ulong)attempt + 1, 90_000);
            if (result.Frame is not null)
            {
                converted = result;
                break;
            }
        }

        for (var attempt = 0; attempt < 120 && converted?.Frame is null; attempt++)
        {
            var result = converter.TryDrain(context);
            if (result.Frame is not null)
            {
                converted = result;
                break;
            }

            Thread.Sleep(1);
        }

        Assert.IsNotNull(converted?.Frame, "边缘像素夹具应在有界尝试内完成 GPU 回读");
        Assert.IsTrue(
            converted!.Frame!.Payload[0] is >= 70 and <= 95,
            "源纹理左上角的红色像素必须映射到输出左上角的 Y；否则 GPU 采样存在半像素偏移");
    }
}
