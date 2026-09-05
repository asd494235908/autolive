using GpAutoLive.Core.Processes;
using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class RtmpEncoderProbeTests
{
    [TestMethod]
    public async Task Selects_first_available_encoder_in_the_configured_order()
    {
        var runner = new FakeRunner("h264_amf");

        var result = await RtmpEncoderProbe.ProbeAsync(
            Environment.ProcessPath,
            null,
            runner);

        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        Assert.AreEqual("h264_amf", result.Snapshot.SelectedEncoder);
        CollectionAssert.AreEqual(
            new[] { "h264_nvenc", "h264_amf" },
            result.Snapshot.Candidates.Select(static candidate => candidate.Encoder).ToArray());
        Assert.IsFalse(result.Snapshot.Candidates[0].IsAvailable);
        Assert.IsTrue(result.Snapshot.Candidates[1].IsAvailable);
    }

    [TestMethod]
    public async Task Preferred_encoder_starts_at_that_candidate_without_fallback_to_higher_priority()
    {
        var runner = new FakeRunner("h264_qsv", "h264_amf");

        var result = await RtmpEncoderProbe.ProbeAsync(
            Environment.ProcessPath,
            "h264_qsv",
            runner);

        Assert.IsTrue(result.IsSuccess, result.Error?.Message);
        Assert.AreEqual("h264_qsv", result.Snapshot.SelectedEncoder);
        CollectionAssert.AreEqual(
            new[] { "h264_qsv" },
            result.Snapshot.Candidates.Select(static candidate => candidate.Encoder).ToArray());
    }

    [TestMethod]
    public async Task Returns_no_encoder_when_all_candidates_fail()
    {
        var runner = new FakeRunner();

        var result = await RtmpEncoderProbe.ProbeAsync(
            Environment.ProcessPath,
            null,
            runner);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(RtmpEncoderProbeFailureCode.NoEncoder, result.Error?.Code);
        Assert.AreEqual(5, result.Snapshot.Candidates.Length);
        Assert.IsNull(result.Snapshot.SelectedEncoder);
    }

    [TestMethod]
    public async Task Rejects_unknown_preferred_encoder_without_running_a_process()
    {
        var runner = new FakeRunner("h264_nvenc");

        var result = await RtmpEncoderProbe.ProbeAsync(
            Environment.ProcessPath,
            "h264_unknown",
            runner);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(RtmpEncoderProbeFailureCode.InvalidArguments, result.Error?.Code);
        Assert.AreEqual(0, runner.Plans.Count);
    }

    private sealed class FakeRunner : IExternalProcessRunner
    {
        private readonly HashSet<string> _available;

        public FakeRunner(params string[] available)
        {
            _available = available.ToHashSet(StringComparer.Ordinal);
        }

        public List<ExternalProcessPlan> Plans { get; } = [];

        public Task<ExternalProcessResult> RunAsync(
            ExternalProcessPlan plan,
            CancellationToken cancellationToken)
        {
            Plans.Add(plan);
            var encoderIndex = plan.Arguments.IndexOf("-c:v");
            var encoder = encoderIndex >= 0 && encoderIndex + 1 < plan.Arguments.Length
                ? plan.Arguments[encoderIndex + 1]
                : string.Empty;
            var success = _available.Contains(encoder);
            return Task.FromResult(new ExternalProcessResult(
                ExternalProcessRunStatus.Completed,
                success ? 0 : 1,
                string.Empty,
                string.Empty));
        }
    }
}
