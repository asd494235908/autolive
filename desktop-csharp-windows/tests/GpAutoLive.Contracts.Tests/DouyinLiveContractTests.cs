using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Contracts.Tests;

[TestClass]
public sealed class DouyinLiveContractTests
{
    [TestMethod]
    public void Config_normalizes_standard_room_url_and_trimmed_replies()
    {
        var config = new DouyinLiveConfig
        {
            Enabled = true,
            RoomId = " https://live.douyin.com/12345 ",
            Replies = [" 你好 ", "收到✅"],
            QueueCapacity = 500
        };

        Assert.IsTrue(DouyinLiveRules.TryNormalize(config, out var normalized, out var error), error?.Message);
        Assert.AreEqual("12345", normalized!.RoomId);
        CollectionAssert.AreEqual(new[] { "你好", "收到✅" }, normalized.Replies.ToArray());
    }

    [TestMethod]
    public void Config_rejects_invalid_room_reply_and_queue_bounds()
    {
        Assert.IsFalse(
            DouyinLiveRules.TryNormalize(DouyinLiveConfig.Default with { RoomId = "http://live.douyin.com/1" }, out _, out var roomError));
        Assert.AreEqual(DouyinLiveConfigFailureCode.RoomIdInvalid, roomError!.Code);

        Assert.IsTrue(
            DouyinLiveRules.TryNormalize(DouyinConfig() with { Replies = ["a", "a"] }, out var deduplicated, out var duplicateError),
            duplicateError?.Message);
        CollectionAssert.AreEqual(new[] { "a" }, deduplicated!.Replies.ToArray());

        Assert.IsFalse(
            DouyinLiveRules.TryNormalize(DouyinConfig() with { QueueCapacity = 9 }, out _, out var queueError));
        Assert.AreEqual(DouyinLiveConfigFailureCode.QueueCapacityOutOfRange, queueError!.Code);
    }

    [TestMethod]
    public void Config_rejects_control_characters_and_utf8_overflow()
    {
        var control = DouyinConfig() with { Replies = ["hello\nworld"] };
        Assert.IsFalse(DouyinLiveRules.TryNormalize(control, out _, out var controlError));
        Assert.AreEqual(DouyinLiveConfigFailureCode.ReplyInvalid, controlError!.Code);

        var tooLarge = DouyinConfig() with { Replies = [new string('界', 81)] };
        Assert.IsFalse(DouyinLiveRules.TryNormalize(tooLarge, out _, out var lengthError));
        Assert.AreEqual(DouyinLiveConfigFailureCode.ReplyInvalid, lengthError!.Code);
    }

    [TestMethod]
    public void Json_contract_uses_snake_case_and_does_not_add_credentials()
    {
        var json = JsonSerializer.Serialize(DouyinConfig(), ContractJson.CreateOptions());

        StringAssert.Contains(json, "\"room_id\":\"12345\"");
        StringAssert.Contains(json, "\"queue_capacity\":500");
        Assert.IsFalse(json.Contains("cookie", StringComparison.OrdinalIgnoreCase));
        Assert.IsFalse(json.Contains("token", StringComparison.OrdinalIgnoreCase));
    }

    private static DouyinLiveConfig DouyinConfig() => new()
    {
        Enabled = true,
        RoomId = "12345",
        Replies = ["收到", "谢谢"],
        QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
    };
}
