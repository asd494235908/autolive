using System.IO;
using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class InterludeStartupRestoreTests
{
    [TestMethod]
    public void Enabled_saved_directory_restores_the_candidate_snapshot_without_starting_playback()
    {
        var directory = Path.Combine(Path.GetTempPath(), $"GpAutoLive.InterludeRestore.{Guid.NewGuid():N}");
        Directory.CreateDirectory(directory);
        File.WriteAllBytes(Path.Combine(directory, "saved.mp3"), [1]);
        try
        {
            var pool = new InterludeFilePoolService();
            var config = InterludeAudioConfig.Default with
            {
                Enabled = true,
                Directory = directory,
            };

            var result = MainWindow.RestoreConfiguredInterludePool(config, pool, CancellationToken.None);

            Assert.IsNotNull(result);
            Assert.IsTrue(result.IsSuccess);
            Assert.AreEqual(InterludePoolStatus.Ready, pool.Snapshot.Status);
            Assert.AreEqual(1, pool.Snapshot.Files.Length);
        }
        finally
        {
            Directory.Delete(directory, recursive: true);
        }
    }

    [TestMethod]
    public void Disabled_saved_directory_is_not_scanned_on_startup()
    {
        var pool = new InterludeFilePoolService();
        var config = InterludeAudioConfig.Default with
        {
            Enabled = false,
            Directory = Path.Combine(Path.GetTempPath(), $"missing-{Guid.NewGuid():N}"),
        };

        var result = MainWindow.RestoreConfiguredInterludePool(config, pool, CancellationToken.None);

        Assert.IsNull(result);
        Assert.AreEqual(InterludePoolStatus.Disabled, pool.Snapshot.Status);
    }

    [TestMethod]
    public void Missing_saved_directory_preserves_the_existing_candidate_snapshot()
    {
        var directory = Path.Combine(Path.GetTempPath(), $"GpAutoLive.InterludeExisting.{Guid.NewGuid():N}");
        Directory.CreateDirectory(directory);
        File.WriteAllBytes(Path.Combine(directory, "existing.mp3"), [1]);
        try
        {
            var pool = new InterludeFilePoolService();
            var existing = pool.ScanDirectory(directory);
            Assert.IsTrue(existing.IsSuccess);

            var config = InterludeAudioConfig.Default with
            {
                Enabled = true,
                Directory = Path.Combine(directory, "missing"),
            };

            var result = MainWindow.RestoreConfiguredInterludePool(config, pool, CancellationToken.None);

            Assert.IsNotNull(result);
            Assert.IsFalse(result.IsSuccess);
            Assert.AreEqual(InterludePoolErrorCode.DirectoryNotFound, result.Error?.Code);
            Assert.AreEqual(existing.Snapshot, pool.Snapshot);
        }
        finally
        {
            Directory.Delete(directory, recursive: true);
        }
    }
}
