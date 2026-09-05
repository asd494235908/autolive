using GpAutoLive.App.Features.Douyin;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class DouyinConfigDraftTests
{
    [TestMethod]
    public void ValidDraftSplitsLinesAndPreservesEnabledFlag()
    {
        var ok = DouyinConfigDraft.TryCreate(
            true,
            " https://live.douyin.com/12345 ",
            " 第一条\r\n\r\n第二条 ",
            "500",
            out var config,
            out var error);

        Assert.IsTrue(ok);
        Assert.IsNull(error);
        Assert.IsNotNull(config);
        Assert.IsTrue(config!.Enabled);
        CollectionAssert.AreEqual(new[] { "第一条", "第二条" }, config.Replies.ToArray());
        Assert.AreEqual("12345", config.RoomId);
    }

    [TestMethod]
    public void DuplicateRepliesAreNormalizedWithoutReordering()
    {
        var ok = DouyinConfigDraft.TryCreate(
            false,
            "12345",
            "A\nA\nB",
            "10",
            out var config,
            out _);

        Assert.IsTrue(ok);
        CollectionAssert.AreEqual(new[] { "A", "B" }, config!.Replies.ToArray());
    }

    [TestMethod]
    public void NonNumericQueueCapacityFailsBeforeManagerStart()
    {
        var ok = DouyinConfigDraft.TryCreate(
            true,
            "12345",
            "A",
            "five",
            out var config,
            out var error);

        Assert.IsFalse(ok);
        Assert.IsNull(config);
        StringAssert.Contains(error, "整数");
    }

    [TestMethod]
    public void InvalidRoomIsReportedFromSharedContract()
    {
        var ok = DouyinConfigDraft.TryCreate(
            true,
            "javascript:12345",
            "A",
            "500",
            out var config,
            out var error);

        Assert.IsFalse(ok);
        Assert.IsNull(config);
        StringAssert.Contains(error, "直播间号");
    }
}
