using System.Collections.Immutable;
using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class MediaPoolServiceTests
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
    public void Supported_extensions_cover_video_audio_and_case_insensitive_mixed_pool()
    {
        var videoExtensions = new[] { ".mp4", ".mov", ".mkv", ".avi", ".webm", ".m4v", ".ts", ".m2ts", ".flv", ".wmv", ".3gp" };
        var audioExtensions = new[] { ".mp3", ".wav", ".m4a", ".aac", ".ogg", ".flac" };

        Assert.AreEqual(11, MediaPoolRules.VideoExtensions.Count);
        Assert.AreEqual(6, MediaPoolRules.AudioExtensions.Count);
        Assert.AreEqual(17, MediaPoolRules.SupportedExtensions.Count);

        foreach (var extension in videoExtensions)
        {
            Assert.IsTrue(MediaPoolRules.TryGetMediaKind($"sample{extension.ToUpperInvariant()}", out var kind));
            Assert.AreEqual(MediaKind.Video, kind);
        }

        foreach (var extension in audioExtensions)
        {
            Assert.IsTrue(MediaPoolRules.TryGetMediaKind($"sample{extension.ToUpperInvariant()}", out var kind));
            Assert.AreEqual(MediaKind.Audio, kind);
        }

        var owner = new MediaPoolOwner();
        var mixed = new[] { CreateMedia("first.MP4"), CreateMedia("second.FLAC") };
        var result = owner.ReplaceAll(mixed);

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(2, result.Snapshot.SourceMediaPool.Length);
        Assert.AreEqual(MediaKind.Video, result.Snapshot.SourceMediaPool[0].MediaKind);
        Assert.AreEqual(MediaKind.Audio, result.Snapshot.SourceMediaPool[1].MediaKind);
    }

    [TestMethod]
    public void Replace_all_and_append_enforce_100_item_limit_atomically()
    {
        var owner = new MediaPoolOwner();
        var initial = Enumerable.Range(0, MediaPoolRules.MaxItems)
            .Select(index => CreateMedia($"media-{index:D3}.mp4"))
            .ToArray();

        var committed = owner.ReplaceAll(initial);
        Assert.IsTrue(committed.IsSuccess);
        Assert.AreEqual(MediaPoolRules.MaxItems, owner.Snapshot.SourceMediaPool.Length);

        var before = owner.Snapshot;
        var rejected = owner.Append([CreateMedia("overflow.mp4")]);

        Assert.IsFalse(rejected.IsSuccess);
        Assert.AreEqual("source_media_pool_too_large", rejected.Error!.Code);
        Assert.AreEqual(before, rejected.Snapshot);
        Assert.AreEqual(before, owner.Snapshot);
    }

    [TestMethod]
    public void Replace_all_replaces_the_pool_without_counting_old_items_again()
    {
        var owner = new MediaPoolOwner();
        var initial = Enumerable.Range(0, MediaPoolRules.MaxItems)
            .Select(index => CreateMedia($"replace-all-{index:D3}.mp4"))
            .ToArray();

        Assert.IsTrue(owner.ReplaceAll(initial).IsSuccess);

        var replacement = owner.ReplaceAll([CreateMedia("replacement.mp4")]);

        Assert.IsTrue(replacement.IsSuccess, replacement.Error?.Message);
        Assert.AreEqual(1, replacement.Snapshot.SourceMediaPool.Length);
        Assert.AreEqual("replacement.mp4", replacement.Snapshot.SourceMediaPool[0].FileName);
    }

    [TestMethod]
    public void Duplicate_paths_are_case_insensitive_and_failed_batch_keeps_old_snapshot()
    {
        var owner = new MediaPoolOwner();
        var first = CreateMedia("duplicate.mp4");
        var committed = owner.ReplaceAll([first]);
        Assert.IsTrue(committed.IsSuccess);
        var before = owner.Snapshot;

        var rejected = owner.Append([CreateMedia(first.SourcePath.ToUpperInvariant())]);

        Assert.IsFalse(rejected.IsSuccess);
        Assert.AreEqual("duplicate_source_media_path", rejected.Error!.Code);
        Assert.AreEqual(before, owner.Snapshot);

        var duplicatePath = CreatePath("duplicate-a.mp3");
        File.WriteAllBytes(duplicatePath, [1, 2, 3]);
        var duplicateBatch = owner.ReplaceAll([
            CreateMedia(duplicatePath),
            CreateMedia(duplicatePath.ToUpperInvariant()),
        ]);
        Assert.IsFalse(duplicateBatch.IsSuccess);
        Assert.AreEqual(before, owner.Snapshot);
    }

    [TestMethod]
    public void Invalid_extension_empty_file_missing_file_and_bad_metadata_are_rejected()
    {
        var owner = new MediaPoolOwner();
        var before = owner.Snapshot;

        var unsupported = owner.ReplaceAll([CreateMedia("not-supported.txt")]);
        Assert.IsFalse(unsupported.IsSuccess);
        Assert.AreEqual("unsupported_source_media_extension", unsupported.Error!.Code);
        Assert.AreEqual(before, owner.Snapshot);

        var emptyPath = CreatePath("empty.mp4");
        File.WriteAllBytes(emptyPath, []);
        var empty = owner.ReplaceAll([CreateMedia(emptyPath)]);
        Assert.IsFalse(empty.IsSuccess);
        Assert.AreEqual("source_media_empty", empty.Error!.Code);
        Assert.AreEqual(before, owner.Snapshot);

        var missing = owner.ReplaceAll([CreateMedia("missing.mp3", fileSizeBytes: 0)]);
        Assert.IsFalse(missing.IsSuccess);
        Assert.AreEqual("source_media_empty", missing.Error!.Code);
        Assert.AreEqual(before, owner.Snapshot);

        var invalidPath = CreatePath("invalid.wav");
        File.WriteAllBytes(invalidPath, [1, 2, 3]);
        var invalid = owner.ReplaceAll([CreateMedia(invalidPath, durationMs: 0)]);
        Assert.IsFalse(invalid.IsSuccess);
        Assert.AreEqual("invalid_source_media_metadata", invalid.Error!.Code);
        Assert.AreEqual(before, owner.Snapshot);

        var reference = CreateMedia("reference.mp4") with
        {
            PlaybackReference = CreatePath("not-the-source.mp4"),
        };
        var invalidReference = owner.ReplaceAll([reference]);
        Assert.IsFalse(invalidReference.IsSuccess);
        Assert.AreEqual("playback_reference_mismatch", invalidReference.Error!.Code);
        Assert.AreEqual(before, owner.Snapshot);

        var mismatchedKind = CreateMedia("kind-mismatch.mp4") with
        {
            MediaKind = MediaKind.Audio,
        };
        var invalidKind = owner.ReplaceAll([mismatchedKind]);
        Assert.IsFalse(invalidKind.IsSuccess);
        Assert.AreEqual("media_kind_mismatch", invalidKind.Error!.Code);
        Assert.AreEqual(before, owner.Snapshot);
    }

    [TestMethod]
    public void Paths_are_normalized_and_path_byte_limit_is_enforced()
    {
        var root = CreateDirectory();
        var nested = Path.Combine(root, "nested");
        Directory.CreateDirectory(nested);
        var actualPath = Path.Combine(root, "normalized.mp4");
        File.WriteAllBytes(actualPath, [1, 2, 3]);
        var pathWithDotSegments = Path.Combine(nested, "..", "normalized.mp4");

        var owner = new MediaPoolOwner();
        var normalized = owner.ReplaceAll([CreateMedia(pathWithDotSegments)]);

        Assert.IsTrue(normalized.IsSuccess);
        Assert.AreEqual(Path.GetFullPath(actualPath), normalized.Snapshot.SourceMediaPool[0].SourcePath);
        Assert.AreEqual("normalized.mp4", normalized.Snapshot.SourceMediaPool[0].FileName);

        var tooLong = Path.Combine(root, $"{new string('x', MediaPoolRules.MaxSourcePathBytes)}.mp4");
        var rejected = owner.ReplaceAll([CreateMedia(tooLong, fileSizeBytes: 1, createFile: false)]);
        Assert.IsFalse(rejected.IsSuccess);
        Assert.AreEqual("source_media_path_too_long", rejected.Error!.Code);
        Assert.AreEqual(Path.GetFullPath(actualPath), owner.Snapshot.SourceMediaPool[0].SourcePath);
    }

    [TestMethod]
    public void Crud_operations_are_atomic_and_preserve_order()
    {
        var owner = new MediaPoolOwner();
        var a = CreateMedia("a.mp4");
        var b = CreateMedia("b.mp3");
        var c = CreateMedia("c.mkv");
        Assert.IsTrue(owner.ReplaceAll([a, b, c]).IsSuccess);

        Assert.IsTrue(owner.MoveDown(0).IsSuccess);
        Assert.AreEqual("b.mp3", owner.Snapshot.SourceMediaPool[0].FileName);
        Assert.IsTrue(owner.MoveUp(2).IsSuccess);
        Assert.AreEqual("c.mkv", owner.Snapshot.SourceMediaPool[1].FileName);

        var beforeInvalidReorder = owner.Snapshot;
        var invalidReorder = owner.Reorder([a.SourcePath, a.SourcePath, c.SourcePath]);
        Assert.IsFalse(invalidReorder.IsSuccess);
        Assert.AreEqual(beforeInvalidReorder, owner.Snapshot);

        var reordered = owner.Reorder([
            owner.Snapshot.SourceMediaPool[2].SourcePath,
            owner.Snapshot.SourceMediaPool[0].SourcePath,
            owner.Snapshot.SourceMediaPool[1].SourcePath]);
        Assert.IsTrue(reordered.IsSuccess);
        Assert.AreEqual("a.mp4", owner.Snapshot.SourceMediaPool[0].FileName);

        Assert.IsTrue(owner.RemovePath(b.SourcePath).IsSuccess);
        Assert.AreEqual(2, owner.Snapshot.SourceMediaPool.Length);
        Assert.IsTrue(owner.Clear().IsSuccess);
        Assert.IsTrue(owner.Snapshot.SourceMediaPool.IsEmpty);
        Assert.AreEqual(PlaybackState.Stopped, owner.Snapshot.PlaybackState);
    }

    [TestMethod]
    public void Pool_edits_only_remove_references_and_never_delete_user_source_files()
    {
        var owner = new MediaPoolOwner();
        var first = CreateMedia("preserve-first.mp4");
        var second = CreateMedia("preserve-second.mp4");
        var replacement = CreateMedia("preserve-replacement.mp4");

        Assert.IsTrue(owner.ReplaceAll([first, second]).IsSuccess);
        Assert.IsTrue(owner.RemoveAt(0).IsSuccess);
        Assert.IsTrue(owner.Clear().IsSuccess);
        Assert.IsTrue(owner.ReplaceAll([first, second]).IsSuccess);
        Assert.IsTrue(owner.ReplaceAll([replacement]).IsSuccess);

        Assert.IsTrue(File.Exists(first.SourcePath));
        Assert.IsTrue(File.Exists(second.SourcePath));
        Assert.IsTrue(File.Exists(replacement.SourcePath));
    }

    [TestMethod]
    public void Playback_state_machine_preserves_ready_playing_paused_stopped_invariants()
    {
        var owner = new MediaPoolOwner();
        Assert.IsFalse(owner.StartPlayback().IsSuccess);
        Assert.IsTrue(owner.ReplaceAll([CreateMedia("state.mp4")]).IsSuccess);
        Assert.AreEqual(PlaybackState.Ready, owner.Snapshot.PlaybackState);

        Assert.IsTrue(owner.StartPlayback().IsSuccess);
        Assert.AreEqual(PlaybackState.Playing, owner.Snapshot.PlaybackState);
        Assert.IsTrue(owner.PausePlayback().IsSuccess);
        Assert.AreEqual(PlaybackState.Paused, owner.Snapshot.PlaybackState);
        Assert.IsTrue(owner.ResumePlayback().IsSuccess);
        Assert.AreEqual(PlaybackState.Playing, owner.Snapshot.PlaybackState);

        var generationBeforeStop = owner.Snapshot.PlaybackGeneration;
        Assert.IsTrue(owner.StopPlayback().IsSuccess);
        Assert.AreEqual(PlaybackState.Stopped, owner.Snapshot.PlaybackState);
        Assert.AreEqual(generationBeforeStop + 1, owner.Snapshot.PlaybackGeneration);
        var beforeResume = owner.Snapshot;
        var resume = owner.ResumePlayback();
        Assert.IsFalse(resume.IsSuccess);
        Assert.AreEqual("invalid_playback_transition", resume.Error!.Code);
        Assert.AreEqual(beforeResume, owner.Snapshot);

        Assert.IsTrue(owner.StartPlayback().IsSuccess);
        Assert.AreEqual(PlaybackState.Playing, owner.Snapshot.PlaybackState);
    }

    [TestMethod]
    public void Single_item_loop_keeps_generation_and_advances_loop_identity_once()
    {
        var owner = new MediaPoolOwner();
        Assert.IsTrue(owner.ReplaceAll([CreateMedia("single.mp4")]).IsSuccess);
        Assert.IsTrue(owner.StartPlayback().IsSuccess);
        var identity = owner.CurrentIdentity;
        var generation = owner.Snapshot.PlaybackGeneration;
        var revision = owner.Snapshot.SourceRevision;

        var completed = owner.CompleteCurrent(
            identity.PlaybackGeneration,
            identity.SourceRevision,
            identity.SourceMediaIndex,
            identity.LoopIndex);

        Assert.IsTrue(completed.IsSuccess);
        Assert.AreEqual(generation, completed.Snapshot.PlaybackGeneration);
        Assert.AreEqual(revision, completed.Snapshot.SourceRevision);
        Assert.AreEqual(0, completed.Snapshot.SourceMediaIndex);
        Assert.AreEqual(1UL, completed.Snapshot.LoopIndex);
        Assert.AreEqual(1UL, completed.Snapshot.PlaybackPoolCycle);

        var duplicate = owner.CompleteCurrent(
            identity.PlaybackGeneration,
            identity.SourceRevision,
            identity.SourceMediaIndex,
            identity.LoopIndex);
        Assert.IsFalse(duplicate.IsSuccess);
        Assert.AreEqual("stale_playback_identity", duplicate.Error!.Code);
        Assert.AreEqual(completed.Snapshot, owner.Snapshot);
    }

    [TestMethod]
    public void Multi_item_loop_changes_source_index_and_generation_then_wraps_pool_cycle()
    {
        var owner = new MediaPoolOwner();
        Assert.IsTrue(owner.ReplaceAll([CreateMedia("first.mp4"), CreateMedia("second.mp3")]).IsSuccess);
        Assert.IsTrue(owner.StartPlayback().IsSuccess);
        var firstIdentity = owner.CurrentIdentity;

        var second = owner.CompleteCurrent(
            firstIdentity.PlaybackGeneration,
            firstIdentity.SourceRevision,
            firstIdentity.SourceMediaIndex,
            firstIdentity.LoopIndex);
        Assert.IsTrue(second.IsSuccess);
        Assert.AreEqual(1, second.Snapshot.SourceMediaIndex);
        Assert.AreEqual(firstIdentity.PlaybackGeneration + 1, second.Snapshot.PlaybackGeneration);
        Assert.AreEqual(0UL, second.Snapshot.LoopIndex);
        Assert.AreEqual(0UL, second.Snapshot.PlaybackPoolCycle);

        var secondIdentity = owner.CurrentIdentity;
        var wrapped = owner.CompleteCurrent(
            secondIdentity.PlaybackGeneration,
            secondIdentity.SourceRevision,
            secondIdentity.SourceMediaIndex,
            secondIdentity.LoopIndex);
        Assert.IsTrue(wrapped.IsSuccess);
        Assert.AreEqual(0, wrapped.Snapshot.SourceMediaIndex);
        Assert.AreEqual(secondIdentity.PlaybackGeneration + 1, wrapped.Snapshot.PlaybackGeneration);
        Assert.AreEqual(1UL, wrapped.Snapshot.PlaybackPoolCycle);
        Assert.AreEqual(0UL, wrapped.Snapshot.LoopIndex);

        var stale = owner.CompleteCurrent(
            firstIdentity.PlaybackGeneration,
            firstIdentity.SourceRevision,
            firstIdentity.SourceMediaIndex,
            firstIdentity.LoopIndex);
        Assert.IsFalse(stale.IsSuccess);
        Assert.AreEqual("stale_playback_identity", stale.Error!.Code);
    }

    [TestMethod]
    public void Mixed_pool_completion_keeps_playing_when_audio_advances_to_video()
    {
        var owner = new MediaPoolOwner();
        Assert.IsTrue(owner.ReplaceAll([
            CreateMedia("audio-first.mp3", mediaKind: MediaKind.Audio),
            CreateMedia("video-second.mp4", mediaKind: MediaKind.Video)]).IsSuccess);
        Assert.IsTrue(owner.StartPlayback().IsSuccess);
        var identity = owner.CurrentIdentity;

        var completed = owner.CompleteCurrent(
            identity.PlaybackGeneration,
            identity.SourceRevision,
            identity.SourceMediaIndex,
            identity.LoopIndex);

        Assert.IsTrue(completed.IsSuccess);
        Assert.AreEqual(1, completed.Snapshot.SourceMediaIndex);
        Assert.AreEqual(PlaybackState.Playing, completed.Snapshot.PlaybackState);
    }

    [TestMethod]
    public void Select_at_changes_only_source_identity_and_preserves_playing_state()
    {
        var owner = new MediaPoolOwner();
        Assert.IsTrue(owner.ReplaceAll([
            CreateMedia("select-first.mp4"),
            CreateMedia("select-second.mp3"),
            CreateMedia("select-third.mkv")]).IsSuccess);
        Assert.IsTrue(owner.StartPlayback().IsSuccess);

        var before = owner.Snapshot;
        var identity = owner.CurrentIdentity;
        var selected = owner.SelectAt(2, identity);

        Assert.IsTrue(selected.IsSuccess);
        Assert.IsTrue(selected.Changed);
        Assert.AreEqual(2, selected.Snapshot.SourceMediaIndex);
        Assert.AreEqual(before.PlaybackGeneration + 1, selected.Snapshot.PlaybackGeneration);
        Assert.AreEqual(before.SourceRevision, selected.Snapshot.SourceRevision);
        Assert.AreEqual(0UL, selected.Snapshot.LoopIndex);
        Assert.AreEqual(before.PlaybackPoolCycle, selected.Snapshot.PlaybackPoolCycle);
        Assert.AreEqual(PlaybackState.Playing, selected.Snapshot.PlaybackState);
        CollectionAssert.AreEqual(before.SourceMediaPool, selected.Snapshot.SourceMediaPool);
    }

    [TestMethod]
    public void Select_at_rejects_empty_pool_and_out_of_range_without_mutation()
    {
        var owner = new MediaPoolOwner();
        var emptyBefore = owner.Snapshot;

        var empty = owner.SelectAt(0, owner.CurrentIdentity);

        Assert.IsFalse(empty.IsSuccess);
        Assert.AreEqual("source_media_pool_empty", empty.Error!.Code);
        Assert.AreEqual(emptyBefore, owner.Snapshot);

        Assert.IsTrue(owner.ReplaceAll([CreateMedia("select-range-a.mp4"), CreateMedia("select-range-b.mp3")]).IsSuccess);
        var before = owner.Snapshot;
        var outOfRange = owner.SelectAt(before.SourceMediaPool.Length, owner.CurrentIdentity);

        Assert.IsFalse(outOfRange.IsSuccess);
        Assert.AreEqual("source_media_index_out_of_range", outOfRange.Error!.Code);
        Assert.AreEqual(before, owner.Snapshot);
    }

    [TestMethod]
    public void Select_at_rejects_stale_identity_without_mutation()
    {
        var owner = new MediaPoolOwner();
        Assert.IsTrue(owner.ReplaceAll([
            CreateMedia("select-stale-a.mp4"),
            CreateMedia("select-stale-b.mp3"),
            CreateMedia("select-stale-c.mkv")]).IsSuccess);
        var originalIdentity = owner.CurrentIdentity;
        Assert.IsTrue(owner.SelectAt(1, originalIdentity).IsSuccess);
        var before = owner.Snapshot;

        var stale = owner.SelectAt(2, originalIdentity);

        Assert.IsFalse(stale.IsSuccess);
        Assert.AreEqual("stale_playback_identity", stale.Error!.Code);
        Assert.AreEqual(before, owner.Snapshot);
    }

    [TestMethod]
    public void Previous_and_next_wrap_without_changing_pool_cycle_or_state()
    {
        var owner = new MediaPoolOwner();
        Assert.IsTrue(owner.ReplaceAll([
            CreateMedia("select-wrap-a.mp4"),
            CreateMedia("select-wrap-b.mp3"),
            CreateMedia("select-wrap-c.mkv")]).IsSuccess);

        var next = owner.Next(owner.CurrentIdentity);
        Assert.IsTrue(next.IsSuccess);
        Assert.AreEqual(1, next.Snapshot.SourceMediaIndex);
        Assert.AreEqual(PlaybackState.Ready, next.Snapshot.PlaybackState);

        var previous = owner.Previous(owner.CurrentIdentity);
        Assert.IsTrue(previous.IsSuccess);
        Assert.AreEqual(0, previous.Snapshot.SourceMediaIndex);

        var wrapped = owner.Previous(owner.CurrentIdentity);
        Assert.IsTrue(wrapped.IsSuccess);
        Assert.AreEqual(2, wrapped.Snapshot.SourceMediaIndex);
        Assert.AreEqual(0UL, wrapped.Snapshot.PlaybackPoolCycle);
        Assert.AreEqual(0UL, wrapped.Snapshot.LoopIndex);
    }

    [TestMethod]
    public void Select_at_preserves_ready_paused_and_stopped_states()
    {
        var owner = new MediaPoolOwner();
        Assert.IsTrue(owner.ReplaceAll([CreateMedia("select-state-a.mp4"), CreateMedia("select-state-b.mp3")]).IsSuccess);

        var ready = owner.SelectAt(1, owner.CurrentIdentity);
        Assert.IsTrue(ready.IsSuccess);
        Assert.AreEqual(PlaybackState.Ready, ready.Snapshot.PlaybackState);

        Assert.IsTrue(owner.StartPlayback().IsSuccess);
        Assert.IsTrue(owner.PausePlayback().IsSuccess);
        var paused = owner.SelectAt(0, owner.CurrentIdentity);
        Assert.IsTrue(paused.IsSuccess);
        Assert.AreEqual(PlaybackState.Paused, paused.Snapshot.PlaybackState);

        Assert.IsTrue(owner.StopPlayback().IsSuccess);
        var stopped = owner.SelectAt(1, owner.CurrentIdentity);
        Assert.IsTrue(stopped.IsSuccess);
        Assert.AreEqual(PlaybackState.Stopped, stopped.Snapshot.PlaybackState);
    }

    private string CreateDirectory()
    {
        var directory = Path.Combine(Path.GetTempPath(), "GpAutoLive.CSharp.MediaPool", Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        _temporaryDirectories.Add(directory);
        return directory;
    }

    private string CreatePath(string fileName)
    {
        var directory = CreateDirectory();
        return Path.Combine(directory, fileName);
    }

    private SourceMediaDto CreateMedia(
        string fileName,
        ulong? fileSizeBytes = null,
        ulong? durationMs = 1_000,
        MediaKind? mediaKind = null,
        bool createFile = true)
    {
        var path = Path.IsPathFullyQualified(fileName) ? fileName : CreatePath(fileName);
        if (createFile && !File.Exists(path))
        {
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllBytes(path, [1, 2, 3]);
        }

        var actualSize = createFile ? (ulong)new FileInfo(path).Length : 0;
        return new SourceMediaDto(
            path,
            path,
            mediaKind ?? (MediaPoolRules.TryGetMediaKind(path, out var detected) ? detected : MediaKind.Video),
            MediaCompatibilityMode.Direct,
            Path.GetFileName(path),
            fileSizeBytes ?? actualSize,
            durationMs,
            null,
            null,
            mediaKind == MediaKind.Audio ? null : 1280,
            mediaKind == MediaKind.Audio ? null : 720,
            mediaKind == MediaKind.Audio ? null : 30,
            mediaKind == MediaKind.Video ? null : 48000,
            mediaKind == MediaKind.Video ? null : (ushort)2,
            mediaKind == MediaKind.Audio ? null : "h264",
            mediaKind == MediaKind.Video ? null : "aac",
            null,
            "disabled");
    }
}
