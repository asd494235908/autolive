using System.Buffers.Binary;
using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

public sealed record WindowsMpvPresentationCaptureResult(
    bool IsSuccess,
    byte[]? PngBytes = null,
    WindowsMpvPlaybackControllerError? Error = null);

public sealed partial class WindowsMpvPlaybackController
{
    private const int MaximumPresentationBytes = 32 * 1024 * 1024;

    public async Task<WindowsMpvPresentationCaptureResult> CapturePresentationAsync(
        MediaPlaybackIdentity identity, CancellationToken cancellationToken = default)
    {
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(5));
        try
        {
            await _serial.WaitAsync(timeout.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return CaptureFailure("保帧操作已取消或超时。", WindowsMpvPlaybackControllerFailureCode.Cancelled);
        }
        try
        {
            if (!TryGetRunning(identity, out var runtime, out _, out var error))
            {
                return new(false, Error: error!.Error);
            }
            return await CapturePresentationCoreAsync(runtime!, identity, timeout.Token).ConfigureAwait(false);
        }
        finally
        {
            _serial.Release();
        }
    }

    private static async Task<WindowsMpvPresentationCaptureResult> CapturePresentationCoreAsync(
        WindowsMpvPlaybackRuntime runtime, MediaPlaybackIdentity identity, CancellationToken cancellationToken)
    {
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(5));
        cancellationToken = timeout.Token;
        var id = Guid.NewGuid();
        var path = MpvIpcCommand.PresentationCapturePath(id);
        var owned = false;
        var ownedDirectory = false;
        var result = CaptureFailure("未能获取播放器画面。");
        try
        {
            var configured = await runtime.DispatchAsync(MpvIpcCommand.GetProperty(MpvIpcProperty.VideoOutputConfigured), identity, cancellationToken).ConfigureAwait(false);
            if (!MpvIpcValueReader.TryReadBoolean(configured.Frame!, out var hasOutput, out _) || !hasOutput)
                return CaptureFailure("播放器尚无有效视频画面。", WindowsMpvPlaybackControllerFailureCode.FirstFrameNotObserved);
            var directory = Path.GetDirectoryName(path)!;
            RejectReparseAncestors(directory);
            if (Directory.Exists(directory)) throw new IOException("Capture identifier already exists.");
            Directory.CreateDirectory(directory);
            ownedDirectory = true;
            RejectReparseAncestors(directory);
            // Keep our CreateNew handle open without delete sharing: the target cannot be
            // replaced by a reparse point while mpv writes it. This is current-user LocalAppData.
            await using (var stream = new FileStream(path, FileMode.CreateNew, FileAccess.ReadWrite,
                FileShare.ReadWrite, 4096, FileOptions.Asynchronous))
            {
                owned = true;
                var capture = await runtime.DispatchAsync(MpvIpcCommand.CapturePresentation(id), identity, cancellationToken)
                    .ConfigureAwait(false);
                if (capture.IsSuccess && stream.Length is > 0 and <= MaximumPresentationBytes)
                {
                    var bytes = new byte[(int)stream.Length];
                    await stream.ReadExactlyAsync(bytes, cancellationToken).ConfigureAwait(false);
                    result = IsBoundedPresentationPng(bytes)
                        ? new(true, bytes)
                        : CaptureFailure("播放器保帧数据格式无效。");
                }
            }
        }
        catch (OperationCanceledException)
        {
            result = CaptureFailure("保帧操作已取消或超时。", WindowsMpvPlaybackControllerFailureCode.Cancelled);
        }
        catch (Exception exception) when (exception is IOException or UnauthorizedAccessException or ArgumentException)
        {
            result = CaptureFailure("播放器保帧文件无法安全访问。");
        }
        finally
        {
            if (ownedDirectory)
            {
                try
                {
                    RejectReparseAncestors(Path.GetDirectoryName(path)!);
                    if (owned) File.Delete(path);
                    // Removing this unique empty parent also prevents a timed-out screenshot
                    // writer from recreating the PNG after cleanup has completed.
                    Directory.Delete(Path.GetDirectoryName(path)!, recursive: false);
                }
                catch (Exception exception) when (exception is IOException or UnauthorizedAccessException)
                {
                    result = CaptureFailure("播放器保帧临时文件清理失败。");
                }
            }
        }
        return result;
    }

    private static void RejectReparseAncestors(string directory)
    {
        for (var current = new DirectoryInfo(directory); current is not null; current = current.Parent)
        {
            if (current.Exists && (current.Attributes & FileAttributes.ReparsePoint) != 0)
                throw new IOException("Unsafe capture directory.");
        }
    }

    internal static bool IsBoundedPresentationPng(ReadOnlySpan<byte> bytes)
    {
        if (bytes.Length is < 45 or > MaximumPresentationBytes
            || !bytes[..8].SequenceEqual(new byte[] { 137, 80, 78, 71, 13, 10, 26, 10 })
            || BinaryPrimitives.ReadUInt32BigEndian(bytes[8..12]) != 13
            || !bytes[12..16].SequenceEqual("IHDR"u8)
            || !bytes[^12..].SequenceEqual(new byte[] { 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130 })) return false;
        var width = BinaryPrimitives.ReadUInt32BigEndian(bytes[16..20]);
        var height = BinaryPrimitives.ReadUInt32BigEndian(bytes[20..24]);
        return width is > 0 and <= 8192 && height is > 0 and <= 8192 && (ulong)width * height <= 16_777_216;
    }

    private static WindowsMpvPresentationCaptureResult CaptureFailure(string message,
        WindowsMpvPlaybackControllerFailureCode code = WindowsMpvPlaybackControllerFailureCode.DispatchFailed) =>
        new(false, Error: new(code, message, true));
}
