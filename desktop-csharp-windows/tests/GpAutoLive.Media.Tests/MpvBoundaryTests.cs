using System.IO.Pipes;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class MpvBoundaryTests
{
    [TestMethod]
    public void ShaderOptionsAreCanonicalAndRejectUnsafeValues()
    {
        var created = MpvShaderOptionsSnapshot.TryCreate(
            [
                new KeyValuePair<string, string>("al_runtime_source_fps", "34"),
                new KeyValuePair<string, string>("al_runtime_random_seed", "12"),
            ],
            out var snapshot,
            out var error);

        Assert.IsTrue(created);
        Assert.IsNull(error);
        Assert.IsNotNull(snapshot);
        Assert.AreEqual("al_runtime_random_seed=12,al_runtime_source_fps=34", snapshot.ToMpvValue());

        Assert.IsFalse(MpvShaderOptionsSnapshot.TryCreate(
            [new KeyValuePair<string, string>("al_bad", "x,y")],
            out _,
            out var invalid));
        Assert.AreEqual(MpvVideoParameterFailureCode.InvalidShaderOptions, invalid?.Code);

        Assert.IsFalse(MpvShaderOptionsSnapshot.TryParse("al_value=NaN", out _, out invalid));
        Assert.AreEqual(MpvVideoParameterFailureCode.InvalidShaderOptions, invalid?.Code);
    }

    [TestMethod]
    public void ShaderOptionsRejectKeysThatTheShaderDoesNotDeclare()
    {
        Assert.IsFalse(
            MpvShaderOptionsSnapshot.TryCreate(
                [new KeyValuePair<string, string>("al_not_declared", "1")],
                out _,
                out var error));
        Assert.AreEqual(MpvVideoParameterFailureCode.InvalidShaderOptions, error?.Code);

        Assert.IsFalse(
            MpvShaderOptionsSnapshot.TryCreate(
                [new KeyValuePair<string, string>("al_runtime_plan_hi", "12")],
                out _,
                out error));
        Assert.AreEqual(MpvVideoParameterFailureCode.InvalidShaderOptions, error?.Code);
    }

    [TestMethod]
    public void Gpu83RejectsSourceFpsOutsideTheShaderContractInsteadOfClamping()
    {
        Assert.IsFalse(
            MpvGpu83ShaderSnapshot.TryCreate(
                VideoEffectParams.Default,
                AdvancedEffectParams.Default,
                sourceFps: 0.5,
                epochStartSeconds: 0,
                randomSeed: 42,
                out _,
                out var error));
        Assert.AreEqual(MpvVideoParameterFailureCode.InvalidShaderOptions, error?.Code);

        Assert.IsFalse(
            MpvGpu83ShaderSnapshot.TryCreate(
                VideoEffectParams.Default,
                AdvancedEffectParams.Default,
                sourceFps: 240.5,
                epochStartSeconds: 0,
                randomSeed: 42,
                out _,
                out error));
        Assert.AreEqual(MpvVideoParameterFailureCode.InvalidShaderOptions, error?.Code);
    }

    [TestMethod]
    public void Gpu83MarksUnconsumedFrameRateFieldsUnavailable()
    {
        var video = VideoEffectParams.Default with
        {
            FrameRateJitterPercent = 1,
            FrameRatePerturbationFrequencyHz = 1,
            FrameRatePerturbationAmplitudeFps = 1,
            FrameRateLockEnabled = true,
        };

        Assert.IsTrue(
            MpvGpu83ShaderSnapshot.TryCreate(
                video,
                AdvancedEffectParams.Default,
                sourceFps: 30,
                epochStartSeconds: 0,
                randomSeed: 42,
                out var snapshot,
                out var error),
            error?.Message);
        Assert.IsNotNull(snapshot);

        foreach (var field in new[]
        {
            "video.frame_rate_jitter_percent",
            "video.frame_rate_perturbation_frequency_hz",
            "video.frame_rate_perturbation_amplitude_fps",
            "video.frame_rate_lock_enabled",
        })
        {
            CollectionAssert.Contains(snapshot!.UnavailableFields.ToArray(), field);
        }

        Assert.IsFalse(snapshot!.ShaderOptions.Values.ContainsKey("al_frame_rate_jitter_percent"));
        Assert.IsFalse(snapshot.ShaderOptions.Values.ContainsKey("al_frame_rate_frequency_hz"));
        Assert.IsFalse(snapshot.ShaderOptions.Values.ContainsKey("al_frame_rate_amplitude_fps"));
        Assert.IsFalse(snapshot.ShaderOptions.Values.ContainsKey("al_frame_rate_lock_enabled"));
    }

    [TestMethod]
    public void Gpu83BaselineColorSnapshotCarriesAllFourShaderParameters()
    {
        Assert.IsTrue(
            MpvVideoEffectSnapshot.TryCreateGpu83BaselineColor(
                brightnessPercent: 25,
                contrastPercent: 150,
                saturationPercent: 80,
                hueRotationDegrees: -10,
                out var snapshot,
                out var error),
            error?.Message);
        Assert.IsNotNull(snapshot);
        Assert.AreEqual(
            "al_brightness_percent=25,al_contrast_percent=150,al_hue_degrees=-10,al_saturation_percent=80",
            snapshot!.ShaderOptions.ToMpvValue());
        Assert.AreEqual(MpvVideoProcessingMode.Gpu83, snapshot.Mode);

        Assert.IsFalse(
            MpvVideoEffectSnapshot.TryCreateGpu83BaselineColor(
                brightnessPercent: 101,
                contrastPercent: 100,
                saturationPercent: 100,
                hueRotationDegrees: 0,
                out _,
                out error));
        Assert.AreEqual(MpvVideoParameterFailureCode.InvalidCpu4Value, error?.Code);
    }

    [TestMethod]
    public void ShaderOptionsCanConfirmTheFixedRuntimeReadback()
    {
        Assert.IsTrue(
            MpvShaderOptionsSnapshot.TryCreateBaselineColor(
                brightnessPercent: 25,
                contrastPercent: 150,
                saturationPercent: 80,
                hueRotationDegrees: -10,
                out var snapshot,
                out var error),
            error?.Message);
        Assert.IsNotNull(snapshot);

        using var matching = JsonDocument.Parse(
            "{\"al_brightness_percent\":\"25\",\"al_contrast_percent\":\"150\",\"al_hue_degrees\":\"-10\",\"al_saturation_percent\":\"80\"}");
        Assert.IsTrue(snapshot!.MatchesMpvReadback(matching.RootElement));

        using var stale = JsonDocument.Parse(
            "{\"al_brightness_percent\":\"26\",\"al_contrast_percent\":\"150\",\"al_hue_degrees\":\"-10\",\"al_saturation_percent\":\"80\"}");
        Assert.IsFalse(snapshot.MatchesMpvReadback(stale.RootElement));
    }

    [TestMethod]
    public void ShaderOptionsCanConfirmMpvStringReadbackAndRejectExtraOptions()
    {
        Assert.IsTrue(
            MpvShaderOptionsSnapshot.TryCreateBaselineColor(
                brightnessPercent: 25,
                contrastPercent: 150,
                saturationPercent: 80,
                hueRotationDegrees: -10,
                out var snapshot,
                out var error),
            error?.Message);
        Assert.IsNotNull(snapshot);

        using var stringReadback = JsonDocument.Parse(
            "\"al_hue_degrees=-10,al_saturation_percent=80,al_contrast_percent=150,al_brightness_percent=25\"");
        Assert.IsTrue(snapshot!.MatchesMpvReadback(stringReadback.RootElement));

        using var numericReadback = JsonDocument.Parse(
            "{\"al_brightness_percent\":25,\"al_contrast_percent\":150.0,\"al_hue_degrees\":-10,\"al_saturation_percent\":80}");
        Assert.IsTrue(snapshot.MatchesMpvReadback(numericReadback.RootElement));

        using var extraReadback = JsonDocument.Parse(
            "\"al_brightness_percent=25,al_contrast_percent=150,al_hue_degrees=-10,al_saturation_percent=80,al_extra=1\"");
        Assert.IsFalse(snapshot.MatchesMpvReadback(extraReadback.RootElement));
    }

    [TestMethod]
    public void Cpu4ReadbackMustContainTheSubmittedValues()
    {
        Assert.IsTrue(
            MpvVideoEffectSnapshot.TryCreate(
                MpvVideoProcessingMode.Cpu4,
                brightnessPercent: 10,
                contrastPercent: 110,
                saturationPercent: 90,
                hueRotationDegrees: 5,
                MpvShaderOptionsSnapshot.Empty,
                out var snapshot,
                out var error),
            error?.Message);
        Assert.IsNotNull(snapshot);

        using var matching = JsonDocument.Parse(
            "[\"@autolive_cpu4:lavfi=[eq=brightness=0.1:contrast=1.1:saturation=0.9,hue=h=5:s=1]\"]");
        Assert.IsTrue(snapshot!.MatchesCpu4Readback(matching.RootElement));

        using var stale = JsonDocument.Parse(
            "[\"@autolive_cpu4:lavfi=[eq=brightness=0:contrast=1.1:saturation=0.9,hue=h=5:s=1]\"]");
        Assert.IsFalse(snapshot.MatchesCpu4Readback(stale.RootElement));
    }

    [TestMethod]
    public void Cpu4InstallChainLabelsTheFiltersUsedByRuntimeUpdates()
    {
        Assert.IsTrue(MpvVideoEffectSnapshot.TryCreate(
            MpvVideoProcessingMode.Cpu4,
            brightnessPercent: 10,
            contrastPercent: 110,
            saturationPercent: 90,
            hueRotationDegrees: 5,
            MpvShaderOptionsSnapshot.Empty,
            out var snapshot,
            out var createError), createError?.Message);
        Assert.IsNotNull(snapshot);

        var command = MpvIpcCommand.InstallCpu4FilterChain(snapshot!);
        Assert.IsTrue(command.TrySerialize(1, out var line, out var serializeError), serializeError?.Message);
        Assert.IsNotNull(line);

        using var document = JsonDocument.Parse(line!);
        var filter = document.RootElement.GetProperty("command")[2].GetString();
        StringAssert.Contains(filter, "@autolive_cpu4:lavfi=[");
        StringAssert.Contains(filter, "eq@autolive_cpu4_eq=");
        StringAssert.Contains(filter, "hue@autolive_cpu4_hue=");
    }

    [TestMethod]
    public void Gpu83FullSnapshotMapsTheContractAndRuntimeScheduleInputs()
    {
        var video = VideoEffectParams.Default with
        {
            BrightnessPercent = 5,
            BlurRadiusPx = 2,
            ContrastPercent = 110,
            HueRotationDegrees = -3,
            SharpenPercent = 8,
            NoisePercent = 2,
            DetailEnhancementPercent = 12,
            PixelScalePercent = 102,
            PixelJitterPx = 1,
            DynamicCropPercent = 1,
            SpaceXOffsetPx = 2,
            SpaceYOffsetPx = -1,
            RotationDegrees = 4,
            VignettePercent = 10,
            HighlightsPercent = 6,
            ShadowsPercent = -4,
            RedChannelLockEnabled = true,
            EdgeSoftnessPercent = 15,
            ImageRepairEnabled = true,
            ImageRepairStrengthPercent = 20,
        };
        var bandWeights = AdvancedEffectParams.Default.BandWeights
            .ToDictionary(pair => pair.Key, pair => pair.Key == 1_110 ? 1.25 : 1.0);
        var advanced = AdvancedEffectParams.Default with
        {
            BandWeights = bandWeights,
            TargetFrequencyHz = 1_110,
            CoreFrequencyHz = 777,
            WaveIntensity = 0.25,
            WaveLevel = 0.1,
            WaveGrainCount = 24,
            DynamicEqThreshold = 12,
            ChannelOffsetPercent = 3,
            FrequencySpaceXOffsetPx = 2,
            FrequencySpaceYOffsetPx = -2,
            RandomGraphicEnabled = true,
            RandomGraphicCount = 3,
            RandomGraphicOpacityPercent = 8,
            RandomGraphicSizePx = 6,
            PictureInPictureEnabled = true,
            PictureInPictureOpacityPercent = 60,
            LocalBlurEnabled = true,
            LocalBlurRegionPercent = 20,
            LocalBlurRadiusPx = 2,
            EdgeFillEnabled = true,
            EdgeFeatherPercent = 12,
            FramePerturbationProbabilityPercent = 5,
            SliceLengthMs = 5_000,
            SliceTriggerIntervalMs = 15_000,
            TransformSmoothingEnabled = true,
            HighlightPerturbationEnabled = true,
            AsynchronousRotationEnabled = true,
            AsynchronousRotationMinDegrees = -1,
            AsynchronousRotationMaxDegrees = 1,
        };

        Assert.IsTrue(
            MpvGpu83ShaderSnapshot.TryCreate(
                video,
                advanced,
                sourceFps: 30,
                epochStartSeconds: 0,
                randomSeed: 42,
                out var gpu,
                out var error),
            error?.Message);
        Assert.IsNotNull(gpu);
        Assert.AreEqual(MpvGpu83ShaderSnapshot.ContractParameterCount, gpu!.Entries.Length);
        Assert.IsFalse(gpu.IsFullyAvailable, "未验证/需要历史纹理字段必须保持显式不可用。");
        CollectionAssert.Contains(gpu.UnavailableFields.ToArray(), "video.color_space_conversion_enabled");
        CollectionAssert.Contains(gpu.UnavailableFields.ToArray(), "advanced.slice_min_length_ms");

        Assert.AreEqual("5", gpu.ShaderOptions.Values["al_brightness_percent"]);
        Assert.AreEqual("2", gpu.ShaderOptions.Values["al_blur_radius_px"]);
        Assert.AreEqual("1", gpu.ShaderOptions.Values["al_red_lock_enabled"]);
        Assert.AreEqual("1.25", gpu.ShaderOptions.Values["al_band_1110"]);
        Assert.AreEqual("1110", gpu.ShaderOptions.Values["al_target_frequency_hz"]);
        Assert.AreEqual("30", gpu.ShaderOptions.Values["al_runtime_source_fps"]);
        Assert.AreEqual("42", gpu.ShaderOptions.Values["al_runtime_random_seed"]);
        Assert.AreEqual("5", gpu.ShaderOptions.Values["al_runtime_frame_probability_percent"]);
        Assert.IsFalse(gpu.ShaderOptions.Values.ContainsKey("al_color_space_enabled"));
        Assert.IsFalse(gpu.ShaderOptions.Values.ContainsKey("al_slice_min_length_ms"));

        Assert.IsTrue(
            MpvVideoEffectSnapshot.TryCreateGpu83(
                video,
                advanced,
                sourceFps: 30,
                epochStartSeconds: 0,
                randomSeed: 42,
                out var snapshot,
                out var snapshotGpu,
                out var snapshotError),
            snapshotError?.Message);
        Assert.IsNotNull(snapshot);
        Assert.IsNotNull(snapshotGpu);
        Assert.IsTrue(gpu.Entries.SequenceEqual(snapshotGpu!.Entries));
        Assert.IsTrue(gpu.UnavailableFields.SequenceEqual(snapshotGpu.UnavailableFields));
        CollectionAssert.AreEquivalent(
            gpu.ShaderOptions.Values.ToArray(),
            snapshot!.ShaderOptions.Values.ToArray());
    }

    [TestMethod]
    public void FullGpu83Capability_requires_the_current_csharp_shader_hash()
    {
        var current = new VerifiedRuntimeResource(
            "gpu83.hook",
            @"C:\runtime\gpu83.hook",
            36_625,
            MpvGpu83ShaderSnapshot.FullShaderSha256);
        var oldBaseline = current with
        {
            SizeBytes = 1_511,
            Sha256 = "6b0b7ac9dc3fef74b38f38f9ff2fd516c6c5711390469b5f30d5c5f82cea8a0d",
        };

        Assert.IsTrue(MpvGpu83ShaderSnapshot.IsFullShaderResource(current));
        Assert.IsFalse(MpvGpu83ShaderSnapshot.IsFullShaderResource(oldBaseline));
        Assert.IsFalse(MpvGpu83ShaderSnapshot.IsFullShaderResource(
            current with { Name = "other.hook" }));
    }

    [TestMethod]
    public void CommandsUseFixedMpvJsonShapeAndBoundedNumbers()
    {
        var source = CreateVideoSource(new MediaPlaybackIdentity(1, 2, 0, 0));
        var command = MpvIpcCommand.LoadFileReplace(source, 1_234);

        Assert.IsTrue(command.TrySerialize(7, out var line, out var error));
        Assert.IsNull(error);
        Assert.IsNotNull(line);

        using var document = JsonDocument.Parse(line);
        var root = document.RootElement;
        Assert.AreEqual(7UL, root.GetProperty("request_id").GetUInt64());
        var values = root.GetProperty("command");
        Assert.AreEqual("loadfile", values[0].GetString());
        Assert.AreEqual(source.MediaPath.CanonicalPath, values[1].GetString());
        Assert.AreEqual("replace", values[2].GetString());
        Assert.AreEqual(-1, values[3].GetInt32());
        Assert.AreEqual("1.234", values[4].GetProperty("start").GetString());

        Assert.IsFalse(
            MpvIpcCommand.SetPlaybackSpeed(double.NaN).TrySerialize(8, out _, out error));
        Assert.AreEqual(MpvIpcFailureCode.InvalidCommand, error?.Code);
        Assert.IsFalse(
            MpvIpcCommand.SeekAbsoluteMs(9_007_199_254_740_992).TrySerialize(9, out _, out error));
        Assert.AreEqual(MpvIpcFailureCode.InvalidCommand, error?.Code);

        Assert.IsTrue(
            MpvIpcCommand.GetProperty(MpvIpcProperty.VideoFilterChain)
                .TrySerialize(10, out var filterLine, out error));
        Assert.IsNull(error);
        using var filterDocument = JsonDocument.Parse(filterLine!);
        Assert.AreEqual("get_property", filterDocument.RootElement.GetProperty("command")[0].GetString());
        Assert.AreEqual("vf", filterDocument.RootElement.GetProperty("command")[1].GetString());

        Assert.IsTrue(
            MpvIpcCommand.GetProperty(MpvIpcProperty.EstimatedFrameNumber)
                .TrySerialize(11, out var frameLine, out error));
        Assert.IsNull(error);
        using var frameDocument = JsonDocument.Parse(frameLine!);
        Assert.AreEqual(
            "estimated-frame-number",
            frameDocument.RootElement.GetProperty("command")[1].GetString());
    }

    [TestMethod]
    public void ResponseParserAcceptsSuccessAndClassifiesPropertyFailureWithoutEcho()
    {
        var success = MpvIpcFrameParser.Parse(
            "{\"error\":\"success\",\"data\":true,\"request_id\":4}",
            expectedRequestId: 4);
        Assert.IsTrue(success.IsSuccess);
        Assert.IsNotNull(success.Frame);
        Assert.IsTrue(MpvIpcValueReader.TryReadBoolean(success.Frame, out var value, out var readError));
        Assert.IsTrue(value);
        Assert.IsNull(readError);

        var propertyError = MpvIpcFrameParser.Parse(
            "{\"error\":\"property unavailable\",\"request_id\":4}",
            expectedRequestId: 4);
        Assert.IsTrue(propertyError.IsSuccess);
        Assert.AreEqual(MpvIpcFailureCode.PropertyUnavailable, propertyError.Frame?.Error?.Code);

        var mismatch = MpvIpcFrameParser.Parse(
            "{\"error\":\"success\",\"request_id\":5}",
            expectedRequestId: 4);
        Assert.IsFalse(mismatch.IsSuccess);
        Assert.AreEqual(MpvIpcFailureCode.ResponseRequestIdMismatch, mismatch.Error?.Code);
        Assert.IsFalse(mismatch.Error?.Message.Contains("5", StringComparison.Ordinal) == true);
    }

    [TestMethod]
    public void EventParserClonesDataAndRejectsUnknownFields()
    {
        var parsed = MpvIpcFrameParser.Parse(
            "{\"event\":\"end-file\",\"reason\":\"eof\",\"data\":{\"x\":1}}",
            expectedRequestId: 1);
        Assert.IsTrue(parsed.IsSuccess);
        Assert.AreEqual(MpvIpcFrameKind.Event, parsed.Frame?.Kind);
        Assert.AreEqual("end-file", parsed.Frame?.EventName);
        Assert.AreEqual(1, parsed.Frame?.Data?.GetProperty("x").GetInt32());

        var unknown = MpvIpcFrameParser.Parse(
            "{\"error\":\"success\",\"request_id\":1,\"path\":\"hidden\"}",
            expectedRequestId: 1);
        Assert.IsFalse(unknown.IsSuccess);
        Assert.AreEqual(MpvIpcFailureCode.UnknownField, unknown.Error?.Code);
    }

    [TestMethod]
    public void SessionKeepsOneActiveSourceAndRejectsStaleResponses()
    {
        var session = new MpvPlaybackSession();
        var firstIdentity = new MediaPlaybackIdentity(1, 1, 0, 0);
        var first = CreateVideoSource(firstIdentity);
        var bound = session.BindSource(first);
        Assert.IsTrue(bound.IsSuccess);
        Assert.AreEqual(MpvSessionState.Ready, bound.Snapshot.State);
        Assert.AreEqual(1, bound.Commands.Length);

        Assert.IsTrue(session.TryCreateRequest(
            11,
            MpvIpcCommand.GetProperty(MpvIpcProperty.PlaybackTime),
            firstIdentity,
            out var request,
            out var requestError));
        Assert.IsNull(requestError);

        var secondIdentity = new MediaPlaybackIdentity(2, 2, 1, 0);
        var replaced = session.BindSource(CreateVideoSource(secondIdentity));
        Assert.IsTrue(replaced.IsSuccess);
        Assert.AreEqual(secondIdentity, session.Snapshot.ActiveSource?.Identity);

        var response = MpvIpcFrameParser.Parse(
            "{\"error\":\"success\",\"request_id\":11,\"data\":0}",
            expectedRequestId: 11);
        Assert.IsFalse(session.AcceptResponse(request, response.Frame, out var staleError));
        Assert.AreEqual(MpvSessionFailureCode.StalePlaybackIdentity, staleError?.Code);
    }

    [TestMethod]
    public async Task GatewaySendsFixedCommandAndAcceptsResponseForCurrentIdentity()
    {
        var endpointPath = $@"\\.\pipe\autolive-gateway-{Guid.NewGuid():N}";
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(endpointPath, out var endpoint, out var endpointError));
        Assert.IsNull(endpointError);
        Assert.IsNotNull(endpoint);

        await using var server = new NamedPipeServerStream(
            endpoint!.PipeName,
            PipeDirection.InOut,
            1,
            PipeTransmissionMode.Byte,
            PipeOptions.Asynchronous);
        await using var gateway = new MpvPlaybackIpcGateway(
            new MpvPlaybackSession(),
            new MpvNamedPipeClient(endpoint, new MpvIpcPipeOptions
            {
                ConnectTimeout = TimeSpan.FromSeconds(2),
                ResponseTimeout = TimeSpan.FromSeconds(2),
                MaxFrameBytes = 4 * 1024,
            }));

        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        Assert.IsTrue(gateway.Session.BindSource(CreateVideoSource(identity)).IsSuccess);
        var serverTask = Task.Run(async () =>
        {
            await server.WaitForConnectionAsync();
            using var reader = new StreamReader(server, leaveOpen: true);
            using var writer = new StreamWriter(server, leaveOpen: true) { AutoFlush = true };
            var requestLine = await reader.ReadLineAsync();
            Assert.IsNotNull(requestLine);
            using var request = JsonDocument.Parse(requestLine!);
            Assert.AreEqual("get_property", request.RootElement.GetProperty("command")[0].GetString());
            await writer.WriteLineAsync("{\"error\":\"success\",\"request_id\":1,\"data\":12.5}");
        });

        Assert.IsTrue((await gateway.ConnectAsync()).IsSuccess);
        var result = await gateway.DispatchAsync(
            MpvIpcCommand.GetProperty(MpvIpcProperty.PlaybackTime),
            identity);

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(1UL, result.Request?.RequestId);
        Assert.AreEqual(12.5, result.Frame?.Data?.GetDouble());
        Assert.IsNull(result.IpcError);
        Assert.IsNull(result.SessionError);
        await serverTask;
    }

    [TestMethod]
    public async Task GatewayRejectsResponseAfterSourceReplacement()
    {
        var endpointPath = $@"\\.\pipe\autolive-gateway-{Guid.NewGuid():N}";
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(endpointPath, out var endpoint, out var endpointError));
        Assert.IsNull(endpointError);
        Assert.IsNotNull(endpoint);

        await using var server = new NamedPipeServerStream(
            endpoint!.PipeName,
            PipeDirection.InOut,
            1,
            PipeTransmissionMode.Byte,
            PipeOptions.Asynchronous);
        await using var gateway = new MpvPlaybackIpcGateway(
            new MpvPlaybackSession(),
            new MpvNamedPipeClient(endpoint, new MpvIpcPipeOptions
            {
                ConnectTimeout = TimeSpan.FromSeconds(2),
                ResponseTimeout = TimeSpan.FromSeconds(2),
                MaxFrameBytes = 4 * 1024,
            }));

        var firstIdentity = new MediaPlaybackIdentity(1, 1, 0, 0);
        var secondIdentity = new MediaPlaybackIdentity(2, 2, 1, 0);
        Assert.IsTrue(gateway.Session.BindSource(CreateVideoSource(firstIdentity)).IsSuccess);
        var requestReceived = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var releaseResponse = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var serverTask = Task.Run(async () =>
        {
            await server.WaitForConnectionAsync();
            using var reader = new StreamReader(server, leaveOpen: true);
            using var writer = new StreamWriter(server, leaveOpen: true) { AutoFlush = true };
            _ = await reader.ReadLineAsync();
            requestReceived.TrySetResult(true);
            await releaseResponse.Task;
            await writer.WriteLineAsync("{\"error\":\"success\",\"request_id\":1,\"data\":12.5}");
        });

        Assert.IsTrue((await gateway.ConnectAsync()).IsSuccess);
        var dispatchTask = gateway.DispatchAsync(
            MpvIpcCommand.GetProperty(MpvIpcProperty.PlaybackTime),
            firstIdentity);
        await requestReceived.Task.WaitAsync(TimeSpan.FromSeconds(2));
        Assert.IsTrue(gateway.Session.BindSource(CreateVideoSource(secondIdentity)).IsSuccess);
        releaseResponse.TrySetResult(true);

        var result = await dispatchTask;

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MpvSessionFailureCode.StalePlaybackIdentity, result.SessionError?.Code);
        Assert.IsNull(result.IpcError);
        await serverTask;
    }

    [TestMethod]
    public async Task GatewaySurfacesMpvCommandFailureAsFailure()
    {
        var endpointPath = $@"\\.\pipe\autolive-gateway-{Guid.NewGuid():N}";
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(endpointPath, out var endpoint, out var endpointError));
        Assert.IsNull(endpointError);
        Assert.IsNotNull(endpoint);

        await using var server = new NamedPipeServerStream(
            endpoint!.PipeName,
            PipeDirection.InOut,
            1,
            PipeTransmissionMode.Byte,
            PipeOptions.Asynchronous);
        await using var gateway = new MpvPlaybackIpcGateway(
            new MpvPlaybackSession(),
            new MpvNamedPipeClient(endpoint, new MpvIpcPipeOptions
            {
                ConnectTimeout = TimeSpan.FromSeconds(2),
                ResponseTimeout = TimeSpan.FromSeconds(2),
                MaxFrameBytes = 4 * 1024,
            }));

        var identity = new MediaPlaybackIdentity(3, 3, 0, 0);
        Assert.IsTrue(gateway.Session.BindSource(CreateVideoSource(identity)).IsSuccess);
        var serverTask = Task.Run(async () =>
        {
            await server.WaitForConnectionAsync();
            using var reader = new StreamReader(server, leaveOpen: true);
            using var writer = new StreamWriter(server, leaveOpen: true) { AutoFlush = true };
            _ = await reader.ReadLineAsync();
            await writer.WriteLineAsync("{\"error\":\"property unavailable\",\"request_id\":1}");
        });

        Assert.IsTrue((await gateway.ConnectAsync()).IsSuccess);
        var result = await gateway.DispatchAsync(
            MpvIpcCommand.GetProperty(MpvIpcProperty.PlaybackTime),
            identity);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MpvIpcFailureCode.PropertyUnavailable, result.IpcError?.Code);
        Assert.IsNull(result.SessionError);
        await serverTask;
    }

    [TestMethod]
    public void EffectSnapshotEmitsAtomicModeSpecificCommands()
    {
        var session = new MpvPlaybackSession();
        var identity = new MediaPlaybackIdentity(3, 4, 0, 0);
        Assert.IsTrue(session.BindSource(CreateVideoSource(identity)).IsSuccess);
        Assert.IsTrue(MpvShaderOptionsSnapshot.TryCreate(
            [new KeyValuePair<string, string>("al_runtime_source_fps", "30")],
            out var options,
            out _));
        Assert.IsTrue(MpvVideoEffectSnapshot.TryCreate(
            MpvVideoProcessingMode.Gpu83,
            0,
            100,
            100,
            0,
            options,
            out var gpu,
            out _));

        var gpuResult = session.UpdateEffects(identity, gpu);
        Assert.IsTrue(gpuResult.IsSuccess);
        Assert.AreEqual(2, gpuResult.Commands.Length);

        Assert.IsTrue(MpvVideoEffectSnapshot.TryCreate(
            MpvVideoProcessingMode.Cpu4,
            25,
            150,
            80,
            -10,
            MpvShaderOptionsSnapshot.Empty,
            out var cpu,
            out _));
        var cpuResult = session.UpdateEffects(identity, cpu);
        Assert.IsTrue(cpuResult.IsSuccess);
        Assert.AreEqual(2, cpuResult.Commands.Length);
        Assert.IsNotNull(cpu);
        Assert.AreEqual(0.25, cpu!.MappedCpu4Value(MpvCpu4Parameter.Brightness), 0.0001);
        Assert.AreEqual(1.5, cpu.MappedCpu4Value(MpvCpu4Parameter.Contrast), 0.0001);
    }

    private static MpvActiveSource CreateVideoSource(MediaPlaybackIdentity identity)
    {
        var path = $@"C:\media\sample-{identity.PlaybackGeneration}.mp4";
        var source = new SourceMediaDto(
            path,
            path,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            "sample.mp4",
            1,
            60_000,
            null,
            null,
            1280,
            720,
            30,
            48_000,
            2,
            "h264",
            "aac",
            null,
            "disabled");
        Assert.IsTrue(MpvActiveSource.TryCreate(source, identity, out var active, out var error));
        Assert.IsNull(error);
        Assert.IsNotNull(active);
        return active;
    }
}
