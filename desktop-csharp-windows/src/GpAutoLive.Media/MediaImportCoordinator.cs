using System.Security;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Core.Processes;

namespace GpAutoLive.Media;

/// <summary>媒体导入提交模式。</summary>
public enum MediaImportOperation
{
    /// <summary>用整批探测结果替换当前播放池。</summary>
    ReplaceAll,

    /// <summary>将整批探测结果追加到当前播放池。</summary>
    Append,

    /// <summary>用单个探测结果替换当前播放池中的一个索引。</summary>
    ReplaceAt,
}

/// <summary>一次媒体导入请求。路径只在当前进程内短暂使用，不会写入日志或配置。</summary>
public sealed record MediaImportRequest(
    MediaImportOperation Operation,
    IReadOnlyList<string?>? Paths,
    int? ReplaceIndex = null);

/// <summary>批量导入失败的稳定分类。</summary>
public enum MediaImportFailureCode
{
    InvalidRequest,
    CandidateCountExceeded,
    AppendWouldExceedPool,
    ReplaceAtRequiresSinglePath,
    ReplaceIndexOutOfRange,
    OperationCancelled,
    ProbeFailed,
    SourceMetadataUnavailable,
    SourceParseFailed,
    PoolCommitRejected,
}

/// <summary>
/// 可安全交给 UI 的导入错误。不会包含路径、文件名、命令行、FFprobe 输出或外部异常正文。
/// </summary>
public sealed record MediaImportError(
    MediaImportFailureCode Code,
    string Message,
    int? ItemIndex = null,
    MediaProbeFailureCode? ProbeCode = null,
    string? PoolErrorCode = null);

/// <summary>媒体导入结果；失败时 Snapshot 始终为提交前的播放池快照。</summary>
public sealed record MediaImportResult(
    bool IsSuccess,
    bool Changed,
    AppState Snapshot,
    int ProbedCount,
    MediaImportError? Error = null)
{
    public static MediaImportResult Succeeded(AppState snapshot, bool changed, int probedCount) =>
        new(true, changed, snapshot, probedCount);

    public static MediaImportResult Failed(
        AppState snapshot,
        int probedCount,
        MediaImportError error) =>
        new(false, false, snapshot, probedCount, error);
}

/// <summary>
/// 逐项探测媒体并在全部成功后一次性提交播放池。
/// </summary>
/// <remarks>
/// 该协调器不访问 UI、不启动进程；FFprobe 进程由注入的 <see cref="FfprobeMediaProbe"/>
/// 及其 <see cref="IExternalProcessRunner"/> 负责。协调器按请求顺序串行探测，保证顺序稳定、
/// 峰值内存有界，并用信号量避免两个导入请求交错提交。
/// </remarks>
public sealed class MediaImportCoordinator : IDisposable
{
    private readonly MediaPoolService _mediaPool;
    private readonly FfprobeMediaProbe _probe;
    private readonly SemaphoreSlim _importGate = new(1, 1);
    private readonly object _lifecycleGate = new();
    private int _disposed;
    private int _operationCount;

    public MediaImportCoordinator(
        MediaPoolService mediaPool,
        FfprobeMediaProbe probe)
    {
        _mediaPool = mediaPool ?? throw new ArgumentNullException(nameof(mediaPool));
        _probe = probe ?? throw new ArgumentNullException(nameof(probe));
    }

    /// <summary>
    /// 按请求顺序逐项探测；任一项失败或取消时，不调用播放池写入操作。
    /// </summary>
    public async Task<MediaImportResult> ImportAsync(
        MediaImportRequest? request,
        CancellationToken cancellationToken = default,
        Func<CancellationToken, Task<bool>>? prepareCommitAsync = null)
    {
        EnterOperation();
        var gateAcquired = false;

        try
        {
            await _importGate.WaitAsync(cancellationToken).ConfigureAwait(false);
            gateAcquired = true;
        }
        catch (OperationCanceledException)
        {
            return MediaImportResult.Failed(
                _mediaPool.Snapshot,
                0,
                CancelledError());
        }

        try
        {
            return await ImportCoreAsync(request, cancellationToken, prepareCommitAsync).ConfigureAwait(false);
        }
        finally
        {
            if (gateAcquired)
            {
                try
                {
                    _importGate.Release();
                }
                catch (ObjectDisposedException)
                {
                    // Disposal is deferred until every in-flight operation exits. This is
                    // only a defensive guard for an unusual host shutdown race.
                }
            }

            ExitOperation();
        }
    }

    public void Dispose()
    {
        var disposeGate = false;
        lock (_lifecycleGate)
        {
            if (_disposed == 0)
            {
                _disposed = 1;
                disposeGate = _operationCount == 0;
            }
        }

        if (disposeGate)
        {
            _importGate.Dispose();
        }
    }

    private void EnterOperation()
    {
        lock (_lifecycleGate)
        {
            ObjectDisposedException.ThrowIf(_disposed != 0, this);
            _operationCount++;
        }
    }

    private void ExitOperation()
    {
        var disposeGate = false;
        lock (_lifecycleGate)
        {
            _operationCount--;
            disposeGate = _disposed != 0 && _operationCount == 0;
        }

        if (disposeGate)
        {
            _importGate.Dispose();
        }
    }

    private async Task<MediaImportResult> ImportCoreAsync(
        MediaImportRequest? request,
        CancellationToken cancellationToken,
        Func<CancellationToken, Task<bool>>? prepareCommitAsync)
    {
        var before = _mediaPool.Snapshot;
        if (request is null)
        {
            return Failure(before, 0, MediaImportFailureCode.InvalidRequest, "媒体导入请求不能为空。");
        }

        if (!Enum.IsDefined(request.Operation))
        {
            return Failure(before, 0, MediaImportFailureCode.InvalidRequest, "媒体导入操作无效。");
        }

        if (request.Paths is null)
        {
            return Failure(before, 0, MediaImportFailureCode.InvalidRequest, "媒体路径列表不能为空。");
        }

        int pathCount;
        try
        {
            pathCount = request.Paths.Count;
        }
        catch (Exception exception) when (
            exception is ArgumentException
            or InvalidOperationException
            or NotSupportedException)
        {
            return Failure(before, 0, MediaImportFailureCode.InvalidRequest, "媒体路径列表无效。");
        }

        if (pathCount > MediaPoolRules.MaxItems)
        {
            return Failure(
                before,
                0,
                MediaImportFailureCode.CandidateCountExceeded,
                $"一次最多导入 {MediaPoolRules.MaxItems} 项媒体。");
        }

        if (!TrySnapshotPaths(request.Paths, pathCount, out var paths))
        {
            return Failure(before, 0, MediaImportFailureCode.InvalidRequest, "媒体路径列表无效。");
        }

        var shapeError = ValidateRequestShape(request, paths, before);
        if (shapeError is not null)
        {
            return MediaImportResult.Failed(before, 0, shapeError);
        }

        if (cancellationToken.IsCancellationRequested)
        {
            return MediaImportResult.Failed(before, 0, CancelledError());
        }

        // ponytail: sequential probing is intentionally O(n) with one in-flight probe;
        // bounded FFprobe output/time and the 100-item pool cap are the upgrade boundary.
        var sources = new List<SourceMediaDto>(paths.Length);
        for (var index = 0; index < paths.Length; index++)
        {
            if (cancellationToken.IsCancellationRequested)
            {
                return MediaImportResult.Failed(before, sources.Count, CancelledError(index));
            }

            var candidate = await ProbeOneAsync(paths[index], index, cancellationToken).ConfigureAwait(false);
            if (candidate.Error is not null)
            {
                return MediaImportResult.Failed(before, sources.Count, candidate.Error);
            }

            sources.Add(candidate.Source!);
        }

        if (cancellationToken.IsCancellationRequested)
        {
            return MediaImportResult.Failed(before, sources.Count, CancelledError());
        }

        if (prepareCommitAsync is not null)
        {
            bool prepared;
            try
            {
                prepared = await prepareCommitAsync(cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                return MediaImportResult.Failed(
                    _mediaPool.Snapshot,
                    sources.Count,
                    CancelledError());
            }
            catch (Exception exception) when (
                exception is InvalidOperationException
                or IOException
                or UnauthorizedAccessException
                or ObjectDisposedException)
            {
                return MediaImportResult.Failed(
                    _mediaPool.Snapshot,
                    sources.Count,
                    new MediaImportError(
                        MediaImportFailureCode.PoolCommitRejected,
                        "媒体输出未能安全停止，未提交新的播放池。"));
            }

            if (!prepared)
            {
                return MediaImportResult.Failed(
                    _mediaPool.Snapshot,
                    sources.Count,
                    new MediaImportError(
                        MediaImportFailureCode.PoolCommitRejected,
                        "媒体输出未能安全停止，未提交新的播放池。"));
            }

            if (cancellationToken.IsCancellationRequested)
            {
                return MediaImportResult.Failed(
                    _mediaPool.Snapshot,
                    sources.Count,
                    CancelledError());
            }
        }

        var committed = request.Operation switch
        {
            MediaImportOperation.ReplaceAll => _mediaPool.ReplaceAll(sources),
            MediaImportOperation.Append => _mediaPool.Append(sources),
            MediaImportOperation.ReplaceAt => _mediaPool.ReplaceAt(request.ReplaceIndex!.Value, sources[0]),
            _ => throw new InvalidOperationException("已校验的媒体导入操作不可达。"),
        };

        if (!committed.IsSuccess)
        {
            return MediaImportResult.Failed(
                before,
                sources.Count,
                new MediaImportError(
                    MediaImportFailureCode.PoolCommitRejected,
                    "媒体播放池提交失败，已保留原播放池。",
                    PoolErrorCode: committed.Error?.Code));
        }

        return MediaImportResult.Succeeded(committed.Snapshot, committed.Changed, sources.Count);
    }

    private async Task<ProbeCandidate> ProbeOneAsync(
        string? path,
        int index,
        CancellationToken cancellationToken)
    {
        FfprobeProbeResult probeResult;
        try
        {
            probeResult = await _probe.ProbeAsync(path, cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return ProbeCandidate.Failed(CancelledError(index));
        }

        if (!probeResult.IsSuccess || probeResult.MediaPath is not ValidatedMediaPath mediaPath)
        {
            var probeError = probeResult.Error;
            if (probeError?.Code == MediaProbeFailureCode.OperationCancelled)
            {
                return ProbeCandidate.Failed(CancelledError(index));
            }

            return ProbeCandidate.Failed(new MediaImportError(
                MediaImportFailureCode.ProbeFailed,
                ProbeMessage(probeError?.Code),
                index,
                probeError?.Code));
        }

        if (string.IsNullOrWhiteSpace(probeResult.ProbeJson))
        {
            return ProbeCandidate.Failed(new MediaImportError(
                MediaImportFailureCode.ProbeFailed,
                ProbeMessage(MediaProbeFailureCode.MalformedProbeOutput),
                index,
                MediaProbeFailureCode.MalformedProbeOutput));
        }

        if (!TryReadFileSize(mediaPath.CanonicalPath, out var fileSizeBytes))
        {
            return ProbeCandidate.Failed(new MediaImportError(
                MediaImportFailureCode.SourceMetadataUnavailable,
                "媒体文件元数据暂时不可读取。",
                index,
                MediaProbeFailureCode.FileNotFound));
        }

        if (!FfprobeSourceParser.TryParse(
                mediaPath,
                fileSizeBytes,
                probeResult.ProbeJson,
                out var source,
                out var parseError)
            || source is null)
        {
            return ProbeCandidate.Failed(new MediaImportError(
                MediaImportFailureCode.SourceParseFailed,
                ProbeMessage(parseError?.Code ?? MediaProbeFailureCode.MalformedProbeOutput),
                index,
                parseError?.Code ?? MediaProbeFailureCode.MalformedProbeOutput));
        }

        return ProbeCandidate.Succeeded(source);
    }

    private static MediaImportError? ValidateRequestShape(
        MediaImportRequest request,
        string?[] paths,
        AppState snapshot)
    {
        if (paths.Length == 0)
        {
            return new MediaImportError(
                MediaImportFailureCode.InvalidRequest,
                "媒体路径列表不能为空。");
        }

        if (paths.Length > MediaPoolRules.MaxItems)
        {
            return new MediaImportError(
                MediaImportFailureCode.CandidateCountExceeded,
                $"一次最多导入 {MediaPoolRules.MaxItems} 项媒体。");
        }

        if (request.Operation == MediaImportOperation.ReplaceAt)
        {
            if (request.ReplaceIndex is not int replaceIndex
                || replaceIndex < 0
                || replaceIndex >= snapshot.SourceMediaPool.Length)
            {
                return new MediaImportError(
                    MediaImportFailureCode.ReplaceIndexOutOfRange,
                    "要替换的媒体索引无效，已保留原播放池。");
            }

            if (paths.Length != 1)
            {
                return new MediaImportError(
                    MediaImportFailureCode.ReplaceAtRequiresSinglePath,
                    "单项替换必须只提供一个媒体路径。");
            }

            return null;
        }

        if (request.ReplaceIndex is not null)
        {
            return new MediaImportError(
                MediaImportFailureCode.InvalidRequest,
                "当前导入操作不接受替换索引。");
        }

        if (request.Operation == MediaImportOperation.Append
            && snapshot.SourceMediaPool.Length + paths.Length > MediaPoolRules.MaxItems)
        {
            return new MediaImportError(
                MediaImportFailureCode.AppendWouldExceedPool,
                $"追加后播放池不能超过 {MediaPoolRules.MaxItems} 项媒体。");
        }

        return null;
    }

    private static bool TrySnapshotPaths(
        IReadOnlyList<string?> input,
        int expectedCount,
        out string?[] paths)
    {
        paths = [];
        if (expectedCount < 0 || expectedCount > MediaPoolRules.MaxItems)
        {
            // Keep allocation bounded before reading a hostile/custom list.
            return false;
        }

        try
        {
            if (input.Count != expectedCount)
            {
                return false;
            }

            paths = new string?[expectedCount];
            for (var index = 0; index < paths.Length; index++)
            {
                paths[index] = input[index];
            }

            return true;
        }
        catch (Exception exception) when (
            exception is ArgumentException
            or InvalidOperationException
            or IndexOutOfRangeException
            or NotSupportedException)
        {
            paths = [];
            return false;
        }
    }

    private static bool TryReadFileSize(string path, out ulong fileSizeBytes)
    {
        fileSizeBytes = 0;
        try
        {
            var info = new FileInfo(path);
            if (!info.Exists || info.Attributes.HasFlag(FileAttributes.Directory) || info.Length < 0)
            {
                return false;
            }

            fileSizeBytes = (ulong)info.Length;
            return true;
        }
        catch (Exception exception) when (
            exception is ArgumentException
            or IOException
            or NotSupportedException
            or UnauthorizedAccessException
            or SecurityException)
        {
            return false;
        }
    }

    private static MediaImportResult Failure(
        AppState snapshot,
        int probedCount,
        MediaImportFailureCode code,
        string message,
        int? itemIndex = null) =>
        MediaImportResult.Failed(snapshot, probedCount, new MediaImportError(code, message, itemIndex));

    private static MediaImportError CancelledError(int? itemIndex = null) =>
        new(
            MediaImportFailureCode.OperationCancelled,
            "媒体导入已取消，已保留原播放池。",
            itemIndex);

    private static string ProbeMessage(MediaProbeFailureCode? code) => code switch
    {
        MediaProbeFailureCode.EmptyPath => "媒体路径为空。",
        MediaProbeFailureCode.PathNotFullyQualified => "媒体路径必须是绝对路径。",
        MediaProbeFailureCode.PathTooLong => "媒体文件路径过长。",
        MediaProbeFailureCode.PathContainsControlCharacter => "媒体文件路径包含无效字符。",
        MediaProbeFailureCode.UnsupportedExtension => "媒体文件格式不在支持范围内。",
        MediaProbeFailureCode.FileNotFound => "媒体文件不存在或不可访问。",
        MediaProbeFailureCode.PathIsDirectory => "媒体路径不是文件。",
        MediaProbeFailureCode.InvalidExecutable => "媒体探测资源配置无效。",
        MediaProbeFailureCode.ProcessStartFailed => "媒体探测程序无法启动。",
        MediaProbeFailureCode.ProcessTimedOut => "媒体探测超时。",
        MediaProbeFailureCode.OperationCancelled => "媒体探测已取消。",
        MediaProbeFailureCode.StandardOutputLimitExceeded => "媒体探测输出超过限制。",
        MediaProbeFailureCode.StandardErrorLimitExceeded => "媒体探测错误输出超过限制。",
        MediaProbeFailureCode.ProcessExitedWithError => "媒体探测程序返回失败。",
        MediaProbeFailureCode.MalformedProbeOutput => "媒体探测元数据格式无效。",
        MediaProbeFailureCode.UnsupportedMediaStream => "媒体流类型或必要参数不受支持。",
        _ => "媒体探测失败。",
    };

    private sealed record ProbeCandidate(SourceMediaDto? Source, MediaImportError? Error)
    {
        public static ProbeCandidate Succeeded(SourceMediaDto source) => new(source, null);

        public static ProbeCandidate Failed(MediaImportError error) => new(null, error);
    }
}
