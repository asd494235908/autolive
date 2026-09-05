using System.Collections.Generic;
using System.IO;
using System.Windows;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using GpAutoLive.Contracts;
using GpAutoLive.Media;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Features.Media;

/// <summary>
/// 媒体池缩略图的有界懒加载投影。缓存只保存临时 JPEG 字节，不进入 Core 播放池快照。
/// </summary>
public sealed class MediaThumbnailCache : IAsyncDisposable
{
    private const int MaxCacheEntries = 64;
    private const long MaxCacheBytes = 16 * 1024 * 1024;
    private readonly object _gate = new();
    private readonly WindowsFfmpegThumbnailExtractor _extractor =
        new(new WindowsExternalProcessRunner());
    private readonly Dictionary<string, CacheEntry> _entries = new(StringComparer.OrdinalIgnoreCase);
    private CancellationTokenSource? _loadCancellation;
    private Task? _loadTask;
    private long _cachedBytes;
    private bool _disposed;

    public void Start(
        IReadOnlyList<MediaListItemViewModel> items,
        VerifiedMediaRuntime? runtime,
        Dispatcher dispatcher,
        CancellationToken applicationCancellation,
        Action? thumbnailReady = null)
    {
        ArgumentNullException.ThrowIfNull(items);
        ArgumentNullException.ThrowIfNull(dispatcher);

        CancellationTokenSource cancellation;
        lock (_gate)
        {
            if (_disposed || runtime is null)
            {
                return;
            }

            _loadCancellation?.Cancel();
            _loadCancellation?.Dispose();
            cancellation = CancellationTokenSource.CreateLinkedTokenSource(applicationCancellation);
            _loadCancellation = cancellation;
            _loadTask = LoadAsync(items, runtime, dispatcher, cancellation.Token, thumbnailReady);
        }
    }

    public async ValueTask DisposeAsync()
    {
        Task? loadTask;
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            _loadCancellation?.Cancel();
            loadTask = _loadTask;
        }

        if (loadTask is not null)
        {
            try
            {
                await loadTask.ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                // 关闭窗口时取消缩略图任务是预期路径。
            }
        }

        lock (_gate)
        {
            _loadCancellation?.Dispose();
            _loadCancellation = null;
            _loadTask = null;
            _entries.Clear();
            _cachedBytes = 0;
        }
    }

    private async Task LoadAsync(
        IReadOnlyList<MediaListItemViewModel> items,
        VerifiedMediaRuntime runtime,
        Dispatcher dispatcher,
        CancellationToken cancellationToken,
        Action? thumbnailReady)
    {
        try
        {
            foreach (var item in items)
            {
                cancellationToken.ThrowIfCancellationRequested();
                if (item.MediaKind is not MediaKind.Video)
                {
                    continue;
                }

                var key = CreateKey(item);
                if (!TryGetCached(key, out var bytes))
                {
                    var result = await _extractor
                        .ExtractAsync(runtime, item.Source, cancellationToken)
                        .ConfigureAwait(false);
                    if (!result.IsSuccess || result.ImageBytes is null)
                    {
                        continue;
                    }

                    bytes = result.ImageBytes;
                    AddCached(key, bytes);
                }

                var image = await dispatcher.InvokeAsync(
                        () => CreateBitmap(bytes),
                        System.Windows.Threading.DispatcherPriority.Background,
                        cancellationToken)
                    .Task.ConfigureAwait(false);
                if (image is not null)
                {
                    await dispatcher.InvokeAsync(
                            () =>
                            {
                                item.SetThumbnail(image);
                                thumbnailReady?.Invoke();
                            },
                            System.Windows.Threading.DispatcherPriority.Background,
                            cancellationToken)
                        .Task.ConfigureAwait(false);
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            // Media pool replacement or window close cancels the old thumbnail projection.
        }
    }

    private bool TryGetCached(string key, out byte[] bytes)
    {
        lock (_gate)
        {
            if (_entries.TryGetValue(key, out var entry))
            {
                entry.LastUsedUtc = DateTime.UtcNow;
                bytes = entry.Bytes;
                return true;
            }
        }

        bytes = [];
        return false;
    }

    private void AddCached(string key, byte[] bytes)
    {
        if (bytes.Length > MaxCacheBytes)
        {
            return;
        }

        lock (_gate)
        {
            if (_entries.TryGetValue(key, out var existing))
            {
                _cachedBytes -= existing.Bytes.Length;
            }

            _entries[key] = new CacheEntry(bytes, DateTime.UtcNow);
            _cachedBytes += bytes.Length;
            while (_entries.Count > MaxCacheEntries || _cachedBytes > MaxCacheBytes)
            {
                var oldest = _entries.MinBy(pair => pair.Value.LastUsedUtc);
                if (oldest.Key is null)
                {
                    break;
                }

                _cachedBytes -= oldest.Value.Bytes.Length;
                _entries.Remove(oldest.Key);
            }
        }
    }

    private static string CreateKey(MediaListItemViewModel item)
    {
        var lastWriteTicks = 0L;
        try
        {
            lastWriteTicks = File.GetLastWriteTimeUtc(item.Source.SourcePath).Ticks;
        }
        catch (IOException)
        {
            // Import validation already owns the readable-path gate; a vanished file simply misses the cache.
        }
        catch (UnauthorizedAccessException)
        {
            // Same fail-closed behavior as above.
        }

        return $"{item.Source.SourcePath}|{item.Source.FileSizeBytes}|{lastWriteTicks}|{item.Source.DurationMs}|192x108";
    }

    private static BitmapImage? CreateBitmap(byte[] bytes)
    {
        try
        {
            using var stream = new MemoryStream(bytes, writable: false);
            var image = new BitmapImage();
            image.BeginInit();
            image.CacheOption = BitmapCacheOption.OnLoad;
            image.StreamSource = stream;
            image.EndInit();
            image.Freeze();
            return image;
        }
        catch (ArgumentException)
        {
            return null;
        }
        catch (IOException)
        {
            return null;
        }
    }

    private sealed class CacheEntry(byte[] bytes, DateTime lastUsedUtc)
    {
        public byte[] Bytes { get; } = bytes;

        public DateTime LastUsedUtc { get; set; } = lastUsedUtc;
    }
}
