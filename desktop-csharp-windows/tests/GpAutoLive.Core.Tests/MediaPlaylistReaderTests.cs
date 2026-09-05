using System.Text;
using System.Text.Json;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class MediaPlaylistReaderTests
{
    private readonly List<string> _temporaryDirectories = [];

    [TestCleanup]
    public void Cleanup()
    {
        foreach (var directory in _temporaryDirectories)
        {
            try
            {
                if (Directory.Exists(directory))
                {
                    Directory.Delete(directory, recursive: true);
                }
            }
            catch (IOException)
            {
                // 测试清理失败不覆盖主断言。
            }
        }
    }

    [TestMethod]
    public async Task Reads_versioned_playlist_in_document_order()
    {
        var first = Path.GetFullPath(Path.Combine(Path.GetTempPath(), "playlist-first.mp4"));
        var second = Path.GetFullPath(Path.Combine(Path.GetTempPath(), "playlist-second.mp3"));
        var path = CreatePath("media-playlist.json");
        var content = JsonSerializer.Serialize(new
        {
            schema_version = 1,
            data = new
            {
                items = new[]
                {
                    new { path = first },
                    new { path = second },
                },
            },
        });
        await File.WriteAllTextAsync(path, content, Encoding.UTF8);

        var actual = await new MediaPlaylistReader(path).ReadPathsAsync();

        CollectionAssert.AreEqual(new[] { first, second }, actual.ToArray());
    }

    [TestMethod]
    public async Task Rejects_unknown_fields_and_future_versions()
    {
        var path = CreatePath("media-playlist.json");
        var reader = new MediaPlaylistReader(path);

        await File.WriteAllTextAsync(
            path,
            "{\"schema_version\":1,\"data\":{\"items\":[{\"path\":\"C:\\\\media\\\\clip.mp4\",\"label\":\"unexpected\"}]}}",
            Encoding.UTF8);
        await ThrowsAsync<ConfigurationValidationException>(() => reader.ReadPathsAsync());

        await File.WriteAllTextAsync(
            path,
            "{\"schema_version\":2,\"data\":{\"items\":[{\"path\":\"C:\\\\media\\\\clip.mp4\"}]}}",
            Encoding.UTF8);
        await ThrowsAsync<ConfigurationValidationException>(() => reader.ReadPathsAsync());
    }

    [TestMethod]
    public async Task Rejects_empty_null_relative_remote_duplicate_and_unsupported_items()
    {
        var path = CreatePath("media-playlist.json");
        var reader = new MediaPlaylistReader(path);
        var cases = new[]
        {
            "{\"schema_version\":1,\"data\":{\"items\":[]}}",
            "{\"schema_version\":1,\"data\":{\"items\":[null]}}",
            "{\"schema_version\":1,\"data\":{\"items\":[{\"path\":\"relative\\\\clip.mp4\"}]}}",
            "{\"schema_version\":1,\"data\":{\"items\":[{\"path\":\"\\\\\\\\server\\\\share\\\\clip.mp4\"}]}}",
            "{\"schema_version\":1,\"data\":{\"items\":[{\"path\":\"C:\\\\media\\\\clip.txt\"}]}}",
            "{\"schema_version\":1,\"data\":{\"items\":[{\"path\":\"C:\\\\media\\\\clip.mp4\"},{\"path\":\"c:\\\\media\\\\clip.mp4\"}]}}",
        };

        foreach (var content in cases)
        {
            await File.WriteAllTextAsync(path, content, Encoding.UTF8);
            await ThrowsAsync<ConfigurationValidationException>(() => reader.ReadPathsAsync());
        }
    }

    [TestMethod]
    public async Task Rejects_playlist_larger_than_pool_limit()
    {
        var path = CreatePath("media-playlist.json");
        var content = JsonSerializer.Serialize(new
        {
            schema_version = 1,
            data = new
            {
                items = Enumerable.Range(0, 101)
                    .Select(index => new { path = $@"C:\media\{index}.mp4" })
                    .ToArray(),
            },
        });
        await File.WriteAllTextAsync(path, content, Encoding.UTF8);

        await ThrowsAsync<ConfigurationValidationException>(() => new MediaPlaylistReader(path).ReadPathsAsync());
    }

    private string CreatePath(string fileName)
    {
        var directory = Path.Combine(Path.GetTempPath(), "GpAutoLive.CSharp.Tests", Guid.NewGuid().ToString("N"));
        _temporaryDirectories.Add(directory);
        Directory.CreateDirectory(directory);
        return Path.Combine(directory, fileName);
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
