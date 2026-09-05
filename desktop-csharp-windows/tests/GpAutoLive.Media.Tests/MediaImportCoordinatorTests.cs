using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Processes;
using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class MediaImportCoordinatorTests
{
    [TestMethod]
    public async Task ReplaceAll_append_and_replaceAt_commit_only_after_each_batch_is_valid()
    {
        using var fixture = MediaFixture.Create();
        var pool = new MediaPoolService();
        var runner = new ScriptedRunner(static (_, plan) => SuccessJsonFor(plan));
        using var coordinator = fixture.CreateCoordinator(pool, runner);

        var replaced = await coordinator.ImportAsync(new(
            MediaImportOperation.ReplaceAll,
            [fixture.VideoA, fixture.AudioA]));

        Assert.IsTrue(replaced.IsSuccess);
        Assert.AreEqual(2, replaced.ProbedCount);
        Assert.AreEqual(2, replaced.Snapshot.SourceMediaPool.Length);
        Assert.AreEqual(PlaybackState.Ready, replaced.Snapshot.PlaybackState);

        var appended = await coordinator.ImportAsync(new(
            MediaImportOperation.Append,
            [fixture.VideoB]));

        Assert.IsTrue(appended.IsSuccess);
        Assert.AreEqual(3, appended.Snapshot.SourceMediaPool.Length);
        Assert.AreEqual(fixture.VideoB, appended.Snapshot.SourceMediaPool[2].SourcePath);

        var replacedAt = await coordinator.ImportAsync(new(
            MediaImportOperation.ReplaceAt,
            [fixture.AudioB],
            ReplaceIndex: 1));

        Assert.IsTrue(replacedAt.IsSuccess);
        Assert.AreEqual(3, replacedAt.Snapshot.SourceMediaPool.Length);
        Assert.AreEqual(fixture.AudioB, replacedAt.Snapshot.SourceMediaPool[1].SourcePath);
        Assert.AreEqual(4, runner.CallCount);
    }

    [TestMethod]
    public async Task A_probe_failure_keeps_the_old_snapshot_and_stops_before_later_items()
    {
        using var fixture = MediaFixture.Create();
        var pool = new MediaPoolService();
        var initialRunner = new ScriptedRunner(static (_, plan) => SuccessJsonFor(plan));
        using var coordinator = fixture.CreateCoordinator(pool, initialRunner);
        Assert.IsTrue((await coordinator.ImportAsync(new(
            MediaImportOperation.ReplaceAll,
            [fixture.VideoA]))).IsSuccess);
        var before = pool.Snapshot;

        var runner = new ScriptedRunner([
            SuccessJsonFor,
            static (_, _) => new ExternalProcessResult(
                ExternalProcessRunStatus.Completed,
                ExitCode: 1,
                StandardOutput: string.Empty,
                StandardError: "not exposed"),
        ]);
        using var failingCoordinator = fixture.CreateCoordinator(pool, runner);

        var result = await failingCoordinator.ImportAsync(new(
            MediaImportOperation.ReplaceAll,
            [fixture.VideoB, fixture.AudioA, fixture.AudioB]));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(1, result.ProbedCount);
        Assert.AreEqual(MediaImportFailureCode.ProbeFailed, result.Error?.Code);
        Assert.AreEqual(1, result.Error?.ItemIndex);
        Assert.AreEqual(MediaProbeFailureCode.ProcessExitedWithError, result.Error?.ProbeCode);
        Assert.AreEqual(before, pool.Snapshot);
        Assert.AreEqual(2, runner.CallCount);
        Assert.IsFalse(result.Error!.Message.Contains(fixture.AudioA, StringComparison.Ordinal));
    }

    [TestMethod]
    public async Task Prepare_commit_runs_after_all_probes_and_not_after_a_probe_failure()
    {
        using var fixture = MediaFixture.Create();
        var pool = new MediaPoolService();
        var runner = new ScriptedRunner(static (_, plan) => SuccessJsonFor(plan));
        using var coordinator = fixture.CreateCoordinator(pool, runner);
        var prepareCalls = 0;

        var succeeded = await coordinator.ImportAsync(
            new(MediaImportOperation.ReplaceAll, [fixture.VideoA, fixture.AudioA]),
            prepareCommitAsync: _ =>
            {
                prepareCalls++;
                Assert.AreEqual(2, runner.CallCount);
                return Task.FromResult(true);
            });

        Assert.IsTrue(succeeded.IsSuccess, succeeded.Error?.Message);
        Assert.AreEqual(1, prepareCalls);

        var failingRunner = new ScriptedRunner([
            SuccessJsonFor,
            static (_, _) => new ExternalProcessResult(
                ExternalProcessRunStatus.Completed,
                ExitCode: 1,
                StandardOutput: string.Empty,
                StandardError: "not exposed"),
        ]);
        using var failingCoordinator = fixture.CreateCoordinator(pool, failingRunner);
        var failedPrepareCalls = 0;
        var failed = await failingCoordinator.ImportAsync(
            new(MediaImportOperation.ReplaceAll, [fixture.VideoB, fixture.AudioB]),
            prepareCommitAsync: _ =>
            {
                failedPrepareCalls++;
                return Task.FromResult(true);
            });

        Assert.IsFalse(failed.IsSuccess);
        Assert.AreEqual(MediaImportFailureCode.ProbeFailed, failed.Error?.Code);
        Assert.AreEqual(0, failedPrepareCalls);
    }

    [TestMethod]
    public async Task Cancellation_stops_following_items_and_preserves_the_old_snapshot()
    {
        using var fixture = MediaFixture.Create();
        var pool = new MediaPoolService();
        var cancellation = new CancellationTokenSource();
        var runner = new ScriptedRunner((call, plan) =>
        {
            if (call == 1)
            {
                cancellation.Cancel();
            }

            return SuccessJsonFor(plan);
        });
        using var coordinator = fixture.CreateCoordinator(pool, runner);

        var result = await coordinator.ImportAsync(new(
            MediaImportOperation.ReplaceAll,
            [fixture.VideoA, fixture.VideoB, fixture.AudioA]),
            cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MediaImportFailureCode.OperationCancelled, result.Error?.Code);
        Assert.AreEqual(1, result.ProbedCount);
        Assert.AreEqual(1, result.Error?.ItemIndex);
        Assert.IsTrue(pool.Snapshot.SourceMediaPool.IsEmpty);
        Assert.AreEqual(1, runner.CallCount);
    }

    [TestMethod]
    public async Task Duplicate_paths_are_rejected_by_the_core_commit_and_do_not_change_the_pool()
    {
        using var fixture = MediaFixture.Create();
        var pool = new MediaPoolService();
        var runner = new ScriptedRunner(static (_, plan) => SuccessJsonFor(plan));
        using var coordinator = fixture.CreateCoordinator(pool, runner);

        var before = pool.Snapshot;
        var result = await coordinator.ImportAsync(new(
            MediaImportOperation.ReplaceAll,
            [fixture.VideoA, fixture.VideoA]));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MediaImportFailureCode.PoolCommitRejected, result.Error?.Code);
        Assert.AreEqual("duplicate_source_media_path", result.Error?.PoolErrorCode);
        Assert.AreEqual(2, result.ProbedCount);
        Assert.AreEqual(before, pool.Snapshot);
    }

    [TestMethod]
    public async Task Bounds_and_request_shape_fail_before_starting_a_probe()
    {
        using var fixture = MediaFixture.Create();
        var pool = new MediaPoolService();
        var runner = new ScriptedRunner(static (_, plan) => SuccessJsonFor(plan));
        using var coordinator = fixture.CreateCoordinator(pool, runner);

        var tooMany = await coordinator.ImportAsync(new(
            MediaImportOperation.ReplaceAll,
            Enumerable.Repeat<string?>(fixture.VideoA, MediaPoolRules.MaxItems + 1).ToArray()));
        Assert.IsFalse(tooMany.IsSuccess);
        Assert.AreEqual(MediaImportFailureCode.CandidateCountExceeded, tooMany.Error?.Code);
        Assert.AreEqual(0, runner.CallCount);

        var invalidReplace = await coordinator.ImportAsync(new(
            MediaImportOperation.ReplaceAt,
            [fixture.VideoA],
            ReplaceIndex: 0));
        Assert.IsFalse(invalidReplace.IsSuccess);
        Assert.AreEqual(MediaImportFailureCode.ReplaceIndexOutOfRange, invalidReplace.Error?.Code);
        Assert.AreEqual(0, runner.CallCount);
    }

    [TestMethod]
    public async Task Append_that_would_exceed_the_pool_limit_fails_before_probing()
    {
        using var fixture = MediaFixture.Create();
        var pool = new MediaPoolService();
        var seedRunner = new ScriptedRunner(static (_, plan) => SuccessJsonFor(plan));
        using (var seedCoordinator = fixture.CreateCoordinator(pool, seedRunner))
        {
            var seedPaths = Enumerable.Range(0, MediaPoolRules.MaxItems)
                .Select(index => fixture.CreateFile($"seed-{index}.mp4"))
                .ToArray();
            var seeded = await seedCoordinator.ImportAsync(new(
                MediaImportOperation.ReplaceAll,
                seedPaths));
            Assert.IsTrue(seeded.IsSuccess);
        }

        var before = pool.Snapshot;
        var runner = new ScriptedRunner(static (_, plan) => SuccessJsonFor(plan));
        using var coordinator = fixture.CreateCoordinator(pool, runner);
        var result = await coordinator.ImportAsync(new(
            MediaImportOperation.Append,
            [fixture.VideoA]));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MediaImportFailureCode.AppendWouldExceedPool, result.Error?.Code);
        Assert.AreEqual(0, result.ProbedCount);
        Assert.AreEqual(0, runner.CallCount);
        Assert.AreEqual(before, pool.Snapshot);
    }

    [TestMethod]
    public async Task Dispose_is_idempotent_and_rejects_new_imports()
    {
        using var fixture = MediaFixture.Create();
        var coordinator = fixture.CreateCoordinator(new MediaPoolService(), new ScriptedRunner(static (_, plan) => SuccessJsonFor(plan)));

        coordinator.Dispose();
        coordinator.Dispose();

        try
        {
            await coordinator.ImportAsync(new(
                MediaImportOperation.ReplaceAll,
                [fixture.VideoA]));
            Assert.Fail("已释放的媒体导入协调器不应接受新请求。");
        }
        catch (ObjectDisposedException)
        {
            // Expected: disposal is fail-closed for future imports.
        }
    }

    private static ExternalProcessResult SuccessJsonFor(ExternalProcessPlan plan) =>
        new(
            ExternalProcessRunStatus.Completed,
            ExitCode: 0,
            StandardOutput: Path.GetExtension(plan.Arguments[^1]).Equals(".mp3", StringComparison.OrdinalIgnoreCase)
                ? AudioJson
                : VideoJson,
            StandardError: string.Empty);

    private static ExternalProcessResult SuccessJsonFor(int _, ExternalProcessPlan plan) =>
        SuccessJsonFor(plan);

    private const string VideoJson = """
        {
          "format": { "duration": "1.5" },
          "streams": [
            { "codec_type": "video", "codec_name": "h264", "width": 1280, "height": 720, "avg_frame_rate": "30/1" },
            { "codec_type": "audio", "codec_name": "aac", "sample_rate": "48000", "channels": 2 }
          ]
        }
        """;

    private const string AudioJson = """
        {
          "format": { "duration": "1.5" },
          "streams": [
            { "codec_type": "audio", "codec_name": "mp3", "sample_rate": "44100", "channels": 2 }
          ]
        }
        """;

    private sealed class ScriptedRunner : IExternalProcessRunner
    {
        private readonly Func<int, ExternalProcessPlan, ExternalProcessResult>[] _steps;

        public ScriptedRunner(Func<int, ExternalProcessPlan, ExternalProcessResult> step)
            : this([step])
        {
        }

        public ScriptedRunner(IEnumerable<Func<int, ExternalProcessPlan, ExternalProcessResult>> steps) =>
            _steps = steps.ToArray();

        public int CallCount { get; private set; }

        public Task<ExternalProcessResult> RunAsync(
            ExternalProcessPlan plan,
            CancellationToken cancellationToken)
        {
            CallCount++;
            var step = _steps[Math.Min(CallCount - 1, _steps.Length - 1)];
            return Task.FromResult(step(CallCount, plan));
        }
    }

    private sealed class MediaFixture : IDisposable
    {
        private MediaFixture(string root)
        {
            Root = root;
            VideoA = CreateFile("video-a.mp4");
            VideoB = CreateFile("video-b.mkv");
            AudioA = CreateFile("audio-a.mp3");
            AudioB = CreateFile("audio-b.mp3");
            FfprobePath = Path.Combine(root, "ffprobe.exe");
        }

        public string Root { get; }
        public string FfprobePath { get; }
        public string VideoA { get; }
        public string VideoB { get; }
        public string AudioA { get; }
        public string AudioB { get; }

        public static MediaFixture Create()
        {
            var root = Path.Combine(Path.GetTempPath(), "gpautolive-media-import", Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(root);
            return new MediaFixture(root);
        }

        public MediaImportCoordinator CreateCoordinator(MediaPoolService pool, IExternalProcessRunner runner) =>
            new(pool, new FfprobeMediaProbe(FfprobePath, runner));

        public void Dispose()
        {
            if (Directory.Exists(Root))
            {
                Directory.Delete(Root, recursive: true);
            }
        }

        public string CreateFile(string name)
        {
            var path = Path.Combine(Root, name);
            File.WriteAllBytes(path, [1, 2, 3]);
            return path;
        }
    }
}
