using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class AudioCyclePrewarmCoordinatorTests
{
    [TestMethod]
    public void Planned_candidate_is_prepared_immediately_regardless_of_distance_or_rate()
    {
        var longPlan = AudioCyclePrewarmCoordinator.CreateCandidatePlan(1, "next", 1_000, 10_000);
        Assert.AreEqual(11_000, longPlan.TargetAbsolutePositionMs);
        Assert.AreEqual(AudioCycleCoordinatorAction.Prepare,
            AudioCyclePrewarmCoordinator.GetAction(longPlan, 1_000, 1));
        Assert.AreEqual(AudioCycleCoordinatorAction.Prepare,
            AudioCyclePrewarmCoordinator.GetAction(longPlan, 1_000, 0.5));

        var distant = AudioCyclePrewarmCoordinator.CreateCandidatePlan(2, "next", 1_000, 120_000);
        Assert.AreEqual(AudioCycleCoordinatorAction.Prepare,
            AudioCyclePrewarmCoordinator.GetAction(distant, 1_000, 2));
    }

    [TestMethod]
    public void Preparing_prepared_and_committing_do_not_repeat_prepare()
    {
        var planned = AudioCyclePrewarmCoordinator.CreateCandidatePlan(2, "next", 1_000, 120_000);
        foreach (var status in new[]
                 {
                     AudioCycleCandidateStatus.Preparing,
                     AudioCycleCandidateStatus.Prepared,
                     AudioCycleCandidateStatus.Committing,
                 })
        {
            var candidate = AudioCyclePrewarmCoordinator.WithStatus(planned, status);
            Assert.AreNotEqual(AudioCycleCoordinatorAction.Prepare,
                AudioCyclePrewarmCoordinator.GetAction(candidate, 1_000, 1));
        }
    }

    [TestMethod]
    public void Only_prepared_candidate_commits_at_target_when_no_audio_is_queued()
    {
        var plan = AudioCyclePrewarmCoordinator.CreateCandidatePlan(3, "next", 0, 5_000);
        var preparing = AudioCyclePrewarmCoordinator.WithStatus(plan, AudioCycleCandidateStatus.Preparing);
        var prepared = AudioCyclePrewarmCoordinator.WithStatus(plan, AudioCycleCandidateStatus.Prepared);

        Assert.AreEqual(AudioCycleCoordinatorAction.None,
            AudioCyclePrewarmCoordinator.GetAction(preparing, 5_000, 1, 0));
        Assert.AreEqual(AudioCycleCoordinatorAction.None,
            AudioCyclePrewarmCoordinator.GetAction(prepared, 4_999, 1, 0));
        Assert.AreEqual(AudioCycleCoordinatorAction.Commit,
            AudioCyclePrewarmCoordinator.GetAction(prepared, 5_000, 1, 0));
    }

    [TestMethod]
    public void Commit_lead_accounts_for_queued_media_time_and_playback_rate()
    {
        var plan = AudioCyclePrewarmCoordinator.CreateCandidatePlan(4, "next", 0, 5_000);
        var prepared = AudioCyclePrewarmCoordinator.WithStatus(plan, AudioCycleCandidateStatus.Prepared);

        Assert.AreEqual(AudioCycleCoordinatorAction.None,
            AudioCyclePrewarmCoordinator.GetAction(prepared, 4_749, 1, 250));
        Assert.AreEqual(AudioCycleCoordinatorAction.Commit,
            AudioCyclePrewarmCoordinator.GetAction(prepared, 4_750, 1, 250));
        Assert.AreEqual(AudioCycleCoordinatorAction.None,
            AudioCyclePrewarmCoordinator.GetAction(prepared, 4_624, 1.5, 250));
        Assert.AreEqual(AudioCycleCoordinatorAction.Commit,
            AudioCyclePrewarmCoordinator.GetAction(prepared, 4_625, 1.5, 250));
    }

    [TestMethod]
    public void Prepared_candidate_expires_after_grace_period_but_committing_waits()
    {
        var plan = AudioCyclePrewarmCoordinator.CreateCandidatePlan(5, "next", 0, 5_000);
        var prepared = AudioCyclePrewarmCoordinator.WithStatus(plan, AudioCycleCandidateStatus.Prepared);
        var committing = AudioCyclePrewarmCoordinator.WithStatus(plan, AudioCycleCandidateStatus.Committing);

        Assert.AreEqual(AudioCycleCoordinatorAction.Expire,
            AudioCyclePrewarmCoordinator.GetAction(prepared, 5_501, 1, 250));
        Assert.AreEqual(AudioCycleCoordinatorAction.None,
            AudioCyclePrewarmCoordinator.GetAction(committing, 5_501, 1, 250));
    }

    [TestMethod]
    public void Invalid_numeric_inputs_are_sanitized_without_unbounded_targets()
    {
        var plan = AudioCyclePrewarmCoordinator.CreateCandidatePlan(6, "next", -10, 0);
        Assert.AreEqual(1, plan.TargetAbsolutePositionMs);
        Assert.AreEqual(AudioCycleCoordinatorAction.Prepare,
            AudioCyclePrewarmCoordinator.GetAction(plan, 0, double.NaN, double.PositiveInfinity));
    }

    [TestMethod]
    public void Candidate_status_updates_are_immutable()
    {
        var plan = AudioCyclePrewarmCoordinator.CreateCandidatePlan(7, "next", 100, 200);
        var updated = AudioCyclePrewarmCoordinator.WithStatus(plan, AudioCycleCandidateStatus.Prepared);

        Assert.AreEqual(AudioCycleCandidateStatus.Planned, plan.Status);
        Assert.AreEqual(AudioCycleCandidateStatus.Prepared, updated.Status);
        Assert.AreEqual(plan.CandidateId, updated.CandidateId);
        Assert.AreEqual(plan.TargetAbsolutePositionMs, updated.TargetAbsolutePositionMs);
    }

    [TestMethod]
    public void Planned_evaluation_is_side_effect_free_and_keeps_opaque_sample()
    {
        var sample = new object();
        var plan = AudioCyclePrewarmCoordinator.CreateCandidatePlan(8, sample, 0, 1_000);

        var firstAction = AudioCyclePrewarmCoordinator.GetAction(plan, 0, 1);
        var secondAction = AudioCyclePrewarmCoordinator.GetAction(plan, 0, 1);

        Assert.AreEqual(AudioCycleCoordinatorAction.Prepare, firstAction);
        Assert.AreEqual(AudioCycleCoordinatorAction.Prepare, secondAction);
        Assert.AreSame(sample, plan.Sample);
        Assert.AreEqual(AudioCycleCandidateStatus.Planned, plan.Status);
    }

}
