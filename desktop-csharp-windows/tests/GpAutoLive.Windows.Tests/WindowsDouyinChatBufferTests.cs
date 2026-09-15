using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsDouyinChatBufferTests
{
    [TestMethod]
    public void Recent_messages_are_bounded_deduplicated_and_isolated_by_session()
    {
        var buffer = new WindowsDouyinChatBuffer();
        buffer.Reset(1);
        for (var i = 0; i < 510; i++)
        {
            buffer.Add(Message(i), "room-session", 1);
        }
        buffer.Add(Message(509), "room-session", 1);
        buffer.Add(Message(999), "old-session", 1);
        buffer.Add(Message(999), "room-session", 2);
        Assert.AreEqual(500, buffer.Snapshot().Length);
        Assert.AreEqual("10", buffer.Snapshot()[0].MessageId);
        buffer.Clear();
        buffer.Add(Message(509), "room-session", 1);
        Assert.IsTrue(buffer.Snapshot().IsEmpty);
        buffer.Reset(2);
        buffer.Add(Message(509), "new-session", 2);
        Assert.AreEqual(1, buffer.Snapshot().Length);
    }

    private static DouyinChatDisplayMessage Message(int id) =>
        new(id.ToString(), DateTimeOffset.UnixEpoch, "昵称", "正文🙂", false);
}
