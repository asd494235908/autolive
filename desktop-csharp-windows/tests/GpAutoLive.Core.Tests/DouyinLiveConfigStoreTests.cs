using GpAutoLive.Contracts;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class DouyinLiveConfigStoreTests
{
    [TestMethod]
    public async Task Store_round_trips_non_sensitive_config_atomically()
    {
        var root = Path.Combine(Path.GetTempPath(), $"gpautolive-douyin-config-{Guid.NewGuid():N}");
        var path = Path.Combine(root, "profiles", "douyin", "default.json");
        try
        {
            var store = new DouyinLiveConfigStore(path);
            var expected = new DouyinLiveConfig
            {
                Enabled = true,
                RoomId = " https://live.douyin.com/12345 ",
                Replies = [" A ", "A", "B"],
                QueueCapacity = 100,
            };

            await store.WriteAsync(expected);
            var actual = await store.ReadAsync();

            Assert.IsTrue(actual.Enabled);
            Assert.AreEqual("12345", actual.RoomId);
            CollectionAssert.AreEqual(new[] { "A", "B" }, actual.Replies.ToArray());
            Assert.AreEqual(100, actual.QueueCapacity);
            StringAssert.Contains(await File.ReadAllTextAsync(path), "schema_version");
            Assert.AreEqual(0, Directory.GetFiles(Path.GetDirectoryName(path)!, "*.tmp").Length);
        }
        finally
        {
            if (Directory.Exists(root))
            {
                Directory.Delete(root, recursive: true);
            }
        }
    }

    [TestMethod]
    public async Task Store_rejects_invalid_config_without_creating_file()
    {
        var root = Path.Combine(Path.GetTempPath(), $"gpautolive-douyin-config-{Guid.NewGuid():N}");
        var path = Path.Combine(root, "default.json");
        try
        {
            var store = new DouyinLiveConfigStore(path);
            await ThrowsAsync<ConfigurationValidationException>(() =>
                store.WriteAsync(new DouyinLiveConfig { RoomId = "not-a-room" }));
            Assert.IsFalse(File.Exists(path));
        }
        finally
        {
            if (Directory.Exists(root))
            {
                Directory.Delete(root, recursive: true);
            }
        }
    }

    private static async Task ThrowsAsync<TException>(Func<Task> action)
        where TException : Exception
    {
        try
        {
            await action();
        }
        catch (TException)
        {
            return;
        }

        Assert.Fail($"应抛出 {typeof(TException).Name}。");
    }
}
