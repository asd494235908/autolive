using System.Windows;
using GpAutoLive.App;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class MediaDropPayloadTests
{
    [TestMethod]
    public void File_drop_paths_keep_native_order()
    {
        var expected = new[]
        {
            @"C:\media\first.mp4",
            @"C:\media\second.mp3",
            @"C:\media\third.mkv",
        };
        var data = new DataObject(DataFormats.FileDrop, expected);

        Assert.IsTrue(MediaDropPayload.HasCandidateFiles(data));
        Assert.IsTrue(MediaDropPayload.TryReadPaths(data, out var actual));
        CollectionAssert.AreEqual(expected, actual);
    }

    [TestMethod]
    public void Empty_or_oversized_file_drop_is_rejected()
    {
        var empty = new DataObject(DataFormats.FileDrop, Array.Empty<string>());
        Assert.IsFalse(MediaDropPayload.HasCandidateFiles(empty));
        Assert.IsFalse(MediaDropPayload.TryReadPaths(empty, out _));

        var oversized = Enumerable.Range(0, 101)
            .Select(index => $@"C:\media\{index}.mp4")
            .ToArray();
        var tooMany = new DataObject(DataFormats.FileDrop, oversized);
        Assert.IsFalse(MediaDropPayload.HasCandidateFiles(tooMany));
        Assert.IsFalse(MediaDropPayload.TryReadPaths(tooMany, out _));
    }

    [TestMethod]
    public void Unsupported_extension_is_rejected_by_candidate_check()
    {
        var data = new DataObject(
            DataFormats.FileDrop,
            new[] { @"C:\media\notes.txt" });

        Assert.IsFalse(MediaDropPayload.HasCandidateFiles(data));
    }

    [TestMethod]
    public void Empty_path_is_rejected_by_candidate_check()
    {
        var data = new DataObject(
            DataFormats.FileDrop,
            new[] { string.Empty });

        Assert.IsFalse(MediaDropPayload.HasCandidateFiles(data));
    }

    [TestMethod]
    public void Directory_candidate_is_rejected_by_candidate_check()
    {
        var data = new DataObject(
            DataFormats.FileDrop,
            new[] { @"C:\media\clips\" });

        Assert.IsFalse(MediaDropPayload.HasCandidateFiles(data));
    }

    [TestMethod]
    public void Mixed_supported_and_unsupported_candidates_are_rejected()
    {
        var data = new DataObject(
            DataFormats.FileDrop,
            new[]
            {
                @"C:\media\clip.mp4",
                @"C:\media\notes.txt",
            });

        Assert.IsFalse(MediaDropPayload.HasCandidateFiles(data));
    }

    [TestMethod]
    public void Supported_nonexistent_path_is_accepted_without_file_io()
    {
        var data = new DataObject(
            DataFormats.FileDrop,
            new[] { @"C:\media\not-yet-created.mp4" });

        Assert.IsTrue(MediaDropPayload.HasCandidateFiles(data));
    }
}
