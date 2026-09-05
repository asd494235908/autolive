using System.Collections.Immutable;
using GpAutoLive.Media;
using GpAutoLive.Contracts;
using System.Diagnostics;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsFfmpegPcmDecoderTests
{
    [TestMethod]
    public async Task Invalid_plan_fails_closed_without_starting_a_process()
    {
        await using var decoder = new WindowsFfmpegPcmDecoder();
        var destination = new AudioPcmRingBuffer(128, 2);

        var result = await decoder.DecodeAsync(null, destination);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsFfmpegPcmDecoderFailureCode.InvalidPlan, result.Error?.Code);
        Assert.IsFalse(result.Snapshot.IsRunning);
    }

    [TestMethod]
    public async Task Cancellation_before_decode_is_bounded_and_does_not_start_a_process()
    {
        await using var decoder = new WindowsFfmpegPcmDecoder();
        var destination = new AudioPcmRingBuffer(128, 2);
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        var result = await decoder.DecodeAsync(null, destination, cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsFfmpegPcmDecoderFailureCode.Cancelled, result.Error?.Code);
        Assert.IsFalse(result.Snapshot.IsRunning);
    }

    [TestMethod]
    public async Task Closed_decoder_rejects_future_decode()
    {
        var decoder = new WindowsFfmpegPcmDecoder();
        await decoder.DisposeAsync();
        var destination = new AudioPcmRingBuffer(128, 2);

        var result = await decoder.DecodeAsync(null, destination);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsFfmpegPcmDecoderFailureCode.Closed, result.Error?.Code);
    }

    [TestMethod]
    public async Task Stderr_beyond_tail_limit_is_drained_until_process_exits()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        using var fixture = CmdFixture.Create(
            "@for /L %%i in (1,1,25000) do @echo stderr-line 1>&2");
        var destination = new AudioPcmRingBuffer(128, 1);
        await using var decoder = new WindowsFfmpegPcmDecoder();

        var result = await decoder.DecodeAsync(fixture.Plan, destination);

        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        Assert.IsFalse(result.Snapshot.IsRunning);
        Assert.IsNull(result.Error);
    }

    [TestMethod]
    public async Task Cancellation_during_continuous_stderr_is_bounded_and_joins_reader()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        using var fixture = CmdFixture.Create(
            ":loop\r\n@echo stderr-line 1>&2\r\n@goto loop");
        var destination = new AudioPcmRingBuffer(128, 1);
        await using var decoder = new WindowsFfmpegPcmDecoder();
        using var cancellation = new CancellationTokenSource(TimeSpan.FromMilliseconds(300));
        var stopwatch = Stopwatch.StartNew();

        var result = await decoder.DecodeAsync(fixture.Plan, destination, cancellation.Token);

        stopwatch.Stop();
        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsFfmpegPcmDecoderFailureCode.Cancelled, result.Error?.Code);
        Assert.IsFalse(result.Snapshot.IsRunning);
        Assert.IsTrue(stopwatch.Elapsed < TimeSpan.FromSeconds(4));
    }

    [TestMethod]
    public async Task Real_ffmpeg_fixture_decodes_into_bounded_ring_when_explicitly_enabled()
    {
        var ffmpeg = Environment.GetEnvironmentVariable("AUTOLIVE_TEST_FFMPEG");
        if (string.IsNullOrWhiteSpace(ffmpeg) || !File.Exists(ffmpeg))
        {
            return;
        }

        var fixture = Path.Combine(Path.GetTempPath(), $"gpalive-decoder-{Guid.NewGuid():N}.wav");
        try
        {
            using (var generator = new Process
            {
                StartInfo = new ProcessStartInfo
                {
                    FileName = ffmpeg,
                    UseShellExecute = false,
                    CreateNoWindow = true,
                    RedirectStandardOutput = true,
                    RedirectStandardError = true,
                }
            })
            {
                generator.StartInfo.ArgumentList.Add("-hide_banner");
                generator.StartInfo.ArgumentList.Add("-loglevel");
                generator.StartInfo.ArgumentList.Add("error");
                generator.StartInfo.ArgumentList.Add("-f");
                generator.StartInfo.ArgumentList.Add("lavfi");
                generator.StartInfo.ArgumentList.Add("-i");
                generator.StartInfo.ArgumentList.Add("sine=frequency=440:sample_rate=48000:duration=0.2");
                generator.StartInfo.ArgumentList.Add("-y");
                generator.StartInfo.ArgumentList.Add(fixture);
                Assert.IsTrue(generator.Start());
                await generator.WaitForExitAsync();
                Assert.AreEqual(0, generator.ExitCode);
            }

            var source = new SourceMediaDto(
                fixture,
                "media://decoder-fixture",
                MediaKind.Audio,
                MediaCompatibilityMode.Direct,
                "fixture.wav",
                (ulong)new FileInfo(fixture).Length,
                200,
                0,
                200,
                null,
                null,
                null,
                48_000,
                1,
                null,
                "pcm_s16le",
                null,
                "not-computed");
            Assert.IsTrue(FfmpegPcmDecodePlanBuilder.TryCreate(ffmpeg, source, 48_000, 2, out var plan, out var planError), planError?.Message);

            var destination = new AudioPcmRingBuffer(capacityFrames: 16_000, channels: 2);
            await using var decoder = new WindowsFfmpegPcmDecoder();
            var result = await decoder.DecodeAsync(plan, destination);

            Assert.IsTrue(result.IsSuccess, result.Error?.Message);
            Assert.IsFalse(result.Snapshot.IsRunning);
            Assert.IsTrue(result.Snapshot.DecodedFrames > 0);
            Assert.IsTrue(destination.Snapshot.AvailableFrames > 0);
        }
        finally
        {
            File.Delete(fixture);
        }
    }

    private sealed class CmdFixture : IDisposable
    {
        private CmdFixture(string directory, FfmpegPcmDecodePlan plan)
        {
            Directory = directory;
            Plan = plan;
        }

        public string Directory { get; }

        public FfmpegPcmDecodePlan Plan { get; }

        public static CmdFixture Create(string scriptBody)
        {
            var directory = Path.Combine(
                Path.GetTempPath(),
                "gpautolive-ffmpeg-decoder-tests",
                Guid.NewGuid().ToString("N"));
            System.IO.Directory.CreateDirectory(directory);
            var scriptPath = Path.Combine(directory, "decoder-fixture.cmd");
            File.WriteAllText(scriptPath, $"@echo off\r\n{scriptBody}\r\n");
            var command = Environment.GetEnvironmentVariable("ComSpec")
                ?? Path.Combine(Environment.SystemDirectory, "cmd.exe");
            var plan = new FfmpegPcmDecodePlan(
                command,
                ImmutableArray.Create("/d", "/q", "/c", scriptPath),
                scriptPath,
                48_000,
                1,
                TimeSpan.FromSeconds(5));
            return new CmdFixture(directory, plan);
        }

        public void Dispose()
        {
            if (System.IO.Directory.Exists(Directory))
            {
                System.IO.Directory.Delete(Directory, recursive: true);
            }
        }
    }
}
