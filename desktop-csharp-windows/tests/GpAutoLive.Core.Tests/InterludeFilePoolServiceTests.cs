using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class InterludeFilePoolServiceTests
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
                // 清理失败不覆盖主体断言。
            }
        }
    }

    [TestMethod]
    public void Scan_directory_is_recursive_sorted_and_ignores_unsupported_files()
    {
        var root = CreateDirectory();
        var nested = Directory.CreateDirectory(Path.Combine(root, "nested"));
        File.WriteAllBytes(Path.Combine(root, "b.MP3"), [1, 2]);
        File.WriteAllBytes(Path.Combine(nested.FullName, "a.MKV"), [3]);
        File.WriteAllText(Path.Combine(root, "skip.txt"), "not media");

        var service = new InterludeFilePoolService();
        var result = service.ScanDirectory(root);

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(InterludePoolStatus.Ready, result.Snapshot.Status);
        Assert.AreEqual(2, result.Snapshot.Files.Length);
        Assert.IsTrue(result.Snapshot.Files[0].Path.EndsWith("b.MP3", StringComparison.OrdinalIgnoreCase));
        Assert.AreEqual(MediaKind.Audio, result.Snapshot.Files[0].MediaKind);
        Assert.IsTrue(result.Snapshot.Files[1].Path.EndsWith("a.MKV", StringComparison.OrdinalIgnoreCase));
        Assert.AreEqual(MediaKind.Video, result.Snapshot.Files[1].MediaKind);
        Assert.AreEqual((ulong)1, result.Snapshot.Files[1].FileSizeBytes);
    }

    [TestMethod]
    public void Failed_scan_keeps_previous_snapshot_atomically()
    {
        var root = CreateDirectory();
        var media = Path.Combine(root, "keep.wav");
        File.WriteAllBytes(media, [1]);
        var service = new InterludeFilePoolService();
        var committed = service.ScanDirectory(root);
        Assert.IsTrue(committed.IsSuccess);

        var before = service.Snapshot;
        var rejected = service.ScanDirectory(Path.Combine(root, "missing"));

        Assert.IsFalse(rejected.IsSuccess);
        Assert.AreEqual(InterludePoolErrorCode.DirectoryNotFound, rejected.Error!.Code);
        Assert.AreEqual(before, rejected.Snapshot);
        Assert.AreEqual(before, service.Snapshot);
    }

    [TestMethod]
    public void Empty_directory_is_successful_but_not_ready()
    {
        var service = new InterludeFilePoolService();
        var result = service.ScanDirectory(CreateDirectory());

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(InterludePoolStatus.Empty, result.Snapshot.Status);
        Assert.AreEqual(0, result.Snapshot.Files.Length);
    }

    [TestMethod]
    public void More_than_one_thousand_supported_files_is_rejected_without_commit()
    {
        var root = CreateDirectory();
        for (var index = 0; index <= InterludePoolRules.MaxItems; index++)
        {
            File.WriteAllBytes(Path.Combine(root, $"clip-{index:D4}.mp3"), [1]);
        }

        var service = new InterludeFilePoolService();
        var result = service.ScanDirectory(root);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(InterludePoolErrorCode.TooManyFiles, result.Error!.Code);
        Assert.AreEqual(InterludePoolStatus.Disabled, result.Snapshot.Status);
        Assert.AreEqual(0, result.Snapshot.Files.Length);
    }

    [TestMethod]
    public void Clear_restores_disabled_empty_snapshot()
    {
        var root = CreateDirectory();
        File.WriteAllBytes(Path.Combine(root, "clip.flac"), [1]);
        var service = new InterludeFilePoolService();
        Assert.IsTrue(service.ScanDirectory(root).IsSuccess);

        var result = service.Clear();

        Assert.IsTrue(result.IsSuccess);
        Assert.IsTrue(result.Changed);
        Assert.AreEqual(InterludePoolStatus.Disabled, result.Snapshot.Status);
        Assert.IsNull(result.Snapshot.Directory);
        Assert.AreEqual(0, result.Snapshot.Files.Length);
    }

    private string CreateDirectory()
    {
        var path = Path.Combine(Path.GetTempPath(), "gpautolive-interlude-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(path);
        _temporaryDirectories.Add(path);
        return path;
    }
}
