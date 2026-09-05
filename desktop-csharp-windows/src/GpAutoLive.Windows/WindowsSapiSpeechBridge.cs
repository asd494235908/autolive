using System.Globalization;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;

namespace GpAutoLive.Windows;

/// <summary>
/// Windows SAPI Automation COM 桥接。COM 对象只在每个操作自己的 STA 线程创建和释放，
/// 主 EXE 不携带语音 DLL；系统语音包仍由 Windows 提供。
/// </summary>
public sealed class WindowsSapiSpeechBridge : IWindowsSpeechBridge
{
    private readonly object _gate = new();
    private SapiSpeechOperation? _active;
    private bool _disposed;

    /// <inheritdoc />
    public WindowsSpeechVoiceCatalogResult GetVoices()
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return WindowsSpeechVoiceCatalogResult.EmptyUnavailable(
                    new(WindowsSpeechFailureCode.Closed, "Windows 本地语音桥接器已关闭。"));
            }
        }

        try
        {
            var progId = Type.GetTypeFromProgID("SAPI.SpVoice", throwOnError: false);
            if (progId is null)
            {
                return WindowsSpeechVoiceCatalogResult.EmptyUnavailable(
                    new(WindowsSpeechFailureCode.SapiUnavailable, "Windows SAPI 当前不可用。", Retryable: true));
            }

            var voice = Activator.CreateInstance(progId);
            if (voice is null)
            {
                return WindowsSpeechVoiceCatalogResult.EmptyUnavailable(
                    new(WindowsSpeechFailureCode.SapiUnavailable, "Windows SAPI 当前不可用。", Retryable: true));
            }

            try
            {
                return WindowsSpeechVoiceCatalogResult.Available(EnumerateVoiceDescriptors(voice));
            }
            finally
            {
                ReleaseCom(voice);
            }
        }
        catch (Exception)
        {
            return WindowsSpeechVoiceCatalogResult.EmptyUnavailable(
                new(WindowsSpeechFailureCode.SapiUnavailable, "Windows SAPI voice 目录当前不可用。", Retryable: true));
        }
    }

    /// <inheritdoc />
    public Task<IWindowsSpeechOperation> StartAsync(
        string text,
        string? voiceKey,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Task.FromCanceled<IWindowsSpeechOperation>(cancellationToken);
        }

        SapiSpeechOperation operation;
        lock (_gate)
        {
            if (_disposed)
            {
                throw new ObjectDisposedException(nameof(WindowsSapiSpeechBridge));
            }

            if (_active is not null)
            {
                operation = SapiSpeechOperation.Failed(
                    text,
                    voiceKey,
                    new(WindowsSpeechFailureCode.AlreadyActive, "Windows 本地语音已有活动操作。"));
            }
            else
            {
                operation = new SapiSpeechOperation(text, voiceKey, OnOperationCompleted);
                _active = operation;
            }
        }

        operation.Start();
        return Task.FromResult<IWindowsSpeechOperation>(operation);
    }

    /// <inheritdoc />
    public async ValueTask DisposeAsync()
    {
        SapiSpeechOperation? active;
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            active = _active;
            _active = null;
        }

        if (active is not null)
        {
            await active.CancelAsync().ConfigureAwait(false);
            await active.DisposeAsync().ConfigureAwait(false);
        }
    }

    private void OnOperationCompleted(SapiSpeechOperation operation)
    {
        lock (_gate)
        {
            if (ReferenceEquals(_active, operation))
            {
                _active = null;
            }
        }
    }

    private static IReadOnlyList<WindowsSpeechVoiceDescriptor> EnumerateVoiceDescriptors(object voice)
    {
        dynamic dynamicVoice = voice;
        object? tokens = null;
        var descriptors = new List<WindowsSpeechVoiceDescriptor>();
        try
        {
            tokens = dynamicVoice.GetVoices(string.Empty, string.Empty);
            dynamic dynamicTokens = tokens!;
            var count = Convert.ToInt32(dynamicTokens.Count, CultureInfo.InvariantCulture);
            for (var index = 0; index < count; index++)
            {
                object? token = null;
                try
                {
                    token = dynamicTokens.Item(index);
                    dynamic dynamicToken = token!;
                    var tokenId = Convert.ToString(dynamicToken.Id, CultureInfo.InvariantCulture);
                    if (string.IsNullOrWhiteSpace(tokenId))
                    {
                        continue;
                    }

                    var language = TryReadAttribute(dynamicToken, "Language");
                    descriptors.Add(new(
                        CreateVoiceKey(tokenId),
                        NormalizeCulture(language)));
                }
                finally
                {
                    ReleaseCom(token);
                }
            }
        }
        finally
        {
            ReleaseCom(tokens);
        }

        return descriptors
            .GroupBy(static descriptor => descriptor.Key, StringComparer.Ordinal)
            .Select(static group => group.First())
            .ToArray();
    }

    private static object? FindVoiceToken(object voice, string voiceKey)
    {
        dynamic dynamicVoice = voice;
        object? tokens = null;
        try
        {
            tokens = dynamicVoice.GetVoices(string.Empty, string.Empty);
            dynamic dynamicTokens = tokens!;
            var count = Convert.ToInt32(dynamicTokens.Count, CultureInfo.InvariantCulture);
            for (var index = 0; index < count; index++)
            {
                object? token = null;
                try
                {
                    token = dynamicTokens.Item(index);
                    dynamic dynamicToken = token!;
                    var tokenId = Convert.ToString(dynamicToken.Id, CultureInfo.InvariantCulture);
                    if (!string.IsNullOrWhiteSpace(tokenId)
                        && string.Equals(CreateVoiceKey(tokenId), voiceKey, StringComparison.Ordinal))
                    {
                        var selected = token;
                        token = null;
                        return selected;
                    }
                }
                finally
                {
                    ReleaseCom(token);
                }
            }

            return null;
        }
        finally
        {
            ReleaseCom(tokens);
        }
    }

    private static string? TryReadAttribute(dynamic token, string name)
    {
        try
        {
            return Convert.ToString(token.GetAttribute(name), CultureInfo.InvariantCulture);
        }
        catch (Exception)
        {
            return null;
        }
    }

    private static string NormalizeCulture(string? language) =>
        language switch
        {
            "804" or "0804" => "zh-CN",
            "409" or "0409" => "en-US",
            "404" or "0404" => "zh-TW",
            _ => "und"
        };

    internal static string CreateVoiceKey(string tokenId)
    {
        var digest = SHA256.HashData(Encoding.UTF8.GetBytes(tokenId));
        return Convert.ToHexString(digest);
    }

    private static void ReleaseCom(object? value)
    {
        if (value is not null && Marshal.IsComObject(value))
        {
            try
            {
                Marshal.FinalReleaseComObject(value);
            }
            catch (InvalidComObjectException)
            {
                // 已释放的 RCW 不再需要额外动作。
            }
        }
    }

    private sealed class SapiSpeechOperation : IWindowsSpeechOperation
    {
        private const int SvsFlagsAsync = 1;
        private const int SvsfPurgeBeforeSpeak = 2;
        // SpeechRunState.SRSInactive = 0; 1/2 mean queued/speaking in SAPI Automation.
        private const int SpeechStateInactive = 0;
        private static readonly TimeSpan JoinTimeout = TimeSpan.FromSeconds(2);
        private readonly string _text;
        private readonly string? _voiceKey;
        private readonly Action<SapiSpeechOperation> _completed;
        private readonly CancellationTokenSource _cancel = new();
        private readonly TaskCompletionSource<WindowsSpeechBridgeStartResult> _started =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly TaskCompletionSource<WindowsSpeechBridgeCompletion> _completion =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly TaskCompletionSource<object?> _threadExited =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        private Thread? _thread;
        private int _disposed;
        private int _cancelDisposed;

        public SapiSpeechOperation(
            string text,
            string? voiceKey,
            Action<SapiSpeechOperation> completed)
        {
            _text = text;
            _voiceKey = voiceKey;
            _completed = completed;
        }

        public Task<WindowsSpeechBridgeStartResult> Started => _started.Task;

        public Task<WindowsSpeechBridgeCompletion> Completion => _completion.Task;

        public static SapiSpeechOperation Failed(
            string text,
            string? voiceKey,
            WindowsSpeechError error)
        {
            var operation = new SapiSpeechOperation(text, voiceKey, static _ => { });
            operation._started.TrySetResult(new(false, error));
            operation._completion.TrySetResult(new(WindowsSpeechBridgeCompletionKind.Failed, error));
            return operation;
        }

        public void Start()
        {
            if (_completion.Task.IsCompleted)
            {
                return;
            }

            try
            {
                _thread = new Thread(Run)
                {
                    IsBackground = true,
                    Name = "GpAutoLive-SAPI"
                };
                _thread.SetApartmentState(ApartmentState.STA);
                _thread.Start();
            }
            catch (Exception)
            {
                CompleteFailed(new(WindowsSpeechFailureCode.BridgeUnavailable, "Windows SAPI 线程无法启动。", Retryable: true));
                _threadExited.TrySetResult(null);
                DisposeCancellationSource();
            }
        }

        public Task CancelAsync(CancellationToken cancellationToken = default)
        {
            if (cancellationToken.IsCancellationRequested)
            {
                return Task.FromCanceled(cancellationToken);
            }

            try
            {
                _cancel.Cancel();
            }
            catch (ObjectDisposedException)
            {
                // Dispose 已完成，操作已在终态边界内。
            }

            return Task.CompletedTask;
        }

        public async ValueTask DisposeAsync()
        {
            if (Interlocked.Exchange(ref _disposed, 1) == 0)
            {
                try
                {
                    _cancel.Cancel();
                }
                catch (ObjectDisposedException)
                {
                }
            }

            if (_thread is { } thread && !ReferenceEquals(thread, Thread.CurrentThread))
            {
                try
                {
                    // 等待 Run 的 finally，而不是只等待 Completion；Completion 先完成时，
                    // STA 线程仍可能正在释放 RCW。CancellationTokenSource 只能在线程退出后释放。
                    await _threadExited.Task.WaitAsync(JoinTimeout).ConfigureAwait(false);
                }
                catch (Exception)
                {
                    // 保持 CTS 存活，避免仍在运行的 STA 线程访问已释放的 token。
                    return;
                }
            }

            DisposeCancellationSource();
        }

        private void Run()
        {
            object? voice = null;
            object? selectedToken = null;
            try
            {
                if (_cancel.IsCancellationRequested)
                {
                    CompleteCancelled();
                    return;
                }

                var progId = Type.GetTypeFromProgID("SAPI.SpVoice", throwOnError: false);
                if (progId is null)
                {
                    CompleteFailed(new(WindowsSpeechFailureCode.SapiUnavailable, "Windows SAPI 当前不可用。", Retryable: true));
                    return;
                }

                voice = Activator.CreateInstance(progId);
                if (voice is null)
                {
                    CompleteFailed(new(WindowsSpeechFailureCode.SapiUnavailable, "Windows SAPI 当前不可用。", Retryable: true));
                    return;
                }

                dynamic dynamicVoice = voice;
                if (_voiceKey is not null)
                {
                    selectedToken = FindVoiceToken(voice, _voiceKey);
                    if (selectedToken is null)
                    {
                        CompleteFailed(new(WindowsSpeechFailureCode.VoiceUnavailable, "选择的 Windows 本地 voice 不可用。"));
                        return;
                    }

                    dynamicVoice.Voice = selectedToken;
                }

                dynamicVoice.Speak(_text, SvsFlagsAsync);
                _started.TrySetResult(new(true));

                var observedSpeaking = false;
                while (true)
                {
                    if (_cancel.IsCancellationRequested)
                    {
                        Purge(dynamicVoice);
                        CompleteCancelled();
                        return;
                    }

                    if (IsDone(dynamicVoice, ref observedSpeaking))
                    {
                        _completion.TrySetResult(new(WindowsSpeechBridgeCompletionKind.Completed));
                        _completed(this);
                        return;
                    }

                    _cancel.Token.WaitHandle.WaitOne(50);
                }
            }
            catch (Exception)
            {
                CompleteFailed(new(WindowsSpeechFailureCode.SpeechFailed, "Windows 本地语音播放失败。"));
            }
            finally
            {
                ReleaseCom(selectedToken);
                ReleaseCom(voice);
                _threadExited.TrySetResult(null);
                DisposeCancellationSource();
            }
        }

        private void DisposeCancellationSource()
        {
            if (Interlocked.Exchange(ref _cancelDisposed, 1) == 0)
            {
                _cancel.Dispose();
            }
        }

        private static bool IsDone(dynamic voice, ref bool observedSpeaking)
        {
            object? status = null;
            try
            {
                status = voice.Status;
                var runningState = Convert.ToInt32(((dynamic)status!).RunningState, CultureInfo.InvariantCulture);
                if (runningState != SpeechStateInactive)
                {
                    observedSpeaking = true;
                    return false;
                }

                return observedSpeaking;
            }
            finally
            {
                ReleaseCom(status);
            }
        }

        private static void Purge(dynamic voice)
        {
            try
            {
                voice.Speak(string.Empty, SvsFlagsAsync | SvsfPurgeBeforeSpeak);
            }
            catch (Exception)
            {
                // 取消的终态不应被清理失败改写；Job/COM 释放仍由 finally 负责。
            }
        }

        private void CompleteCancelled()
        {
            _started.TrySetResult(new(false, new(WindowsSpeechFailureCode.Cancelled, "Windows 本地语音已取消。", Retryable: true)));
            _completion.TrySetResult(new(WindowsSpeechBridgeCompletionKind.Cancelled));
            _completed(this);
        }

        private void CompleteFailed(WindowsSpeechError error)
        {
            _started.TrySetResult(new(false, error));
            _completion.TrySetResult(new(WindowsSpeechBridgeCompletionKind.Failed, error));
            _completed(this);
        }
    }
}
