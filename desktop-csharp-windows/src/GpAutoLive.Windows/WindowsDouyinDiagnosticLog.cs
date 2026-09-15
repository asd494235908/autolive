using System.Text.Json;
using System.Text.Json.Serialization;
using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>仅追加结构化白名单记录；没有原始输出或自由文本入口。</summary>
internal sealed class WindowsDouyinDiagnosticLog
{
    internal const int MaximumFileBytes = 1024 * 1024;
    private const int MaximumDiagnosticsPerRun = 64;
    private static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web)
    {
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull
    };
    private readonly object _gate = new();
    private readonly string _directory;
    private readonly int _maximumBytes;
    private string? _writtenPath;
    private string? _runId;
    private int _diagnosticCount;
    private HostState? _lastHost;
    private WindowsDouyinDiagnosticLogState _state;

    internal WindowsDouyinDiagnosticLog()
        : this(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "GpAutoLive", "logs", "douyin"))
    {
    }

    // 仅测试可以替换目录和容量；生产调用方没有任意路径入口。
    internal WindowsDouyinDiagnosticLog(string directory, int maximumBytes = MaximumFileBytes)
    {
        _directory = directory;
        _maximumBytes = maximumBytes;
    }

    internal (string? Path, WindowsDouyinDiagnosticLogState State) Snapshot
    {
        get { lock (_gate) { return (_writtenPath, _state); } }
    }

    internal void BeginRun()
    {
        lock (_gate)
        {
            _runId = Guid.NewGuid().ToString("N");
            _diagnosticCount = 0;
            _lastHost = null;
            Write(new Entry(DateTimeOffset.UtcNow, _runId, "process_starting"));
        }
    }

    internal void RecordHost(WindowsDouyinProbeHostSnapshot snapshot)
    {
        var host = new HostState(snapshot.State, snapshot.Douyin.State, snapshot.ProcessId, snapshot.ExitCode, snapshot.Authenticated, snapshot.LoginClearReason);
        lock (_gate)
        {
            if (_runId is null || host == _lastHost) return;
            _lastHost = host;
            Write(new Entry(DateTimeOffset.UtcNow, _runId, "host_state", State: host.State.ToString(),
                CoreState: host.CoreState.ToString(),
                ProcessId: host.ProcessId, ExitCode: host.ExitCode, Authenticated: host.Authenticated,
                ClearReason: host.ClearReason.ToString()));
        }
    }

    internal void RecordDiagnostic(WindowsDouyinAuthDiagnostic? diagnostic)
    {
        lock (_gate)
        {
            if (_runId is null || _diagnosticCount > MaximumDiagnosticsPerRun) return;
            _diagnosticCount++;
            if (_diagnosticCount > MaximumDiagnosticsPerRun)
            {
                Write(new Entry(DateTimeOffset.UtcNow, _runId, "diagnostic_limit"));
                return;
            }
            if (diagnostic is not { IsValid: true })
            {
                Write(new Entry(DateTimeOffset.UtcNow, _runId, "diagnostic_rejected"));
                return;
            }
            Write(new Entry(DateTimeOffset.UtcNow, _runId, "auth.diagnostic", Stage: diagnostic.Stage,
                Code: diagnostic.Code, ExceptionType: diagnostic.ExceptionType,
                HttpStatus: diagnostic.HttpStatus, PlatformCode: diagnostic.PlatformCode));
        }
    }

    private void Write(Entry entry)
    {
        try
        {
            if (!Path.IsPathFullyQualified(_directory)) throw new IOException();
            RejectReparsePoints(_directory);
            Directory.CreateDirectory(_directory);
            var path = Path.Combine(_directory, "login.ndjson");
            var backup = Path.Combine(_directory, "login.previous.ndjson");
            RejectReparsePoints(path);
            RejectReparsePoints(backup);
            var bytes = JsonSerializer.SerializeToUtf8Bytes(entry, JsonOptions);
            if (bytes.Length + 1 > _maximumBytes) throw new IOException();
            if (File.Exists(path) && new FileInfo(path).Length + bytes.Length + 1 > _maximumBytes)
            {
                File.Move(path, backup, overwrite: true);
            }
            using (var stream = new FileStream(path, FileMode.Append, FileAccess.Write, FileShare.Read))
            {
                stream.Write(bytes);
                stream.WriteByte((byte)'\n');
            }
            _writtenPath = path;
            _state = WindowsDouyinDiagnosticLogState.Ready;
        }
        catch (Exception exception) when (exception is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException or System.Security.SecurityException)
        {
            _state = WindowsDouyinDiagnosticLogState.WriteFailed;
        }
    }

    private static void RejectReparsePoints(string path)
    {
        for (var current = path; !string.IsNullOrEmpty(current); current = Path.GetDirectoryName(current))
        {
            if ((File.Exists(current) || Directory.Exists(current))
                && (File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0)
            {
                throw new IOException();
            }
        }
    }

    private sealed record HostState(WindowsDouyinProbeHostState State, DouyinLiveState CoreState, int? ProcessId, int? ExitCode,
        bool Authenticated, WindowsDouyinLoginClearReason ClearReason);

    private sealed record Entry(DateTimeOffset Timestamp, string RunId, string Event,
        string? Stage = null, string? Code = null, string? ExceptionType = null, int? HttpStatus = null,
        int? PlatformCode = null, string? State = null, string? CoreState = null, int? ProcessId = null, int? ExitCode = null,
        bool? Authenticated = null, string? ClearReason = null);
}
