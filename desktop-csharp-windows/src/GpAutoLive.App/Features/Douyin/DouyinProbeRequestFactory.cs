using System.IO;
using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Features.Douyin;

/// <summary>从显式 Windows 环境变量构造抖音 sidecar 请求；默认未配置时不启用外部进程。</summary>
public static class DouyinProbeRequestFactory
{
    private const string RootVariable = "AUTOLIVE_DOUYIN_ROOT";
    private const string ProbeVariable = "AUTOLIVE_DOUYIN_PROBE";
    private const string CondaVariable = "CONDA_EXE";
    private const string EnvironmentVariable = "AUTOLIVE_CONDA_ENV";
    private const string TimeoutVariable = "AUTOLIVE_DOUYIN_TIMEOUT_SEC";

    /// <summary>
    /// 读取 sidecar 运行配置。三个路径变量全部为空时返回未配置；只设置部分路径时返回脱敏错误。
    /// </summary>
    public static bool TryCreate(
        DouyinLiveConfig config,
        Func<string, string?>? readEnvironment,
        string? temporaryDirectory,
        out WindowsDouyinProbeLaunchRequest? request,
        out string? error,
        out bool configured)
    {
        request = null;
        error = null;
        configured = false;
        ArgumentNullException.ThrowIfNull(config);
        readEnvironment ??= Environment.GetEnvironmentVariable;

        var upstreamRoot = readEnvironment(RootVariable)?.Trim();
        var probeScript = readEnvironment(ProbeVariable)?.Trim();
        var condaExecutable = readEnvironment(CondaVariable)?.Trim();
        configured = !string.IsNullOrWhiteSpace(upstreamRoot)
            || !string.IsNullOrWhiteSpace(probeScript)
            || !string.IsNullOrWhiteSpace(condaExecutable);
        if (!configured)
        {
            return false;
        }

        if (string.IsNullOrWhiteSpace(upstreamRoot)
            || string.IsNullOrWhiteSpace(probeScript)
            || string.IsNullOrWhiteSpace(condaExecutable))
        {
            error = "已配置抖音 sidecar 环境，但 AUTOLIVE_DOUYIN_ROOT、AUTOLIVE_DOUYIN_PROBE、CONDA_EXE 必须同时提供。";
            return false;
        }

        var condaEnvironment = readEnvironment(EnvironmentVariable)?.Trim();
        if (string.IsNullOrWhiteSpace(condaEnvironment))
        {
            condaEnvironment = "gpautolive-douyin";
        }

        var timeout = TimeSpan.FromSeconds(300);
        var timeoutText = readEnvironment(TimeoutVariable)?.Trim();
        if (!string.IsNullOrWhiteSpace(timeoutText)
            && (!int.TryParse(timeoutText, System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out var timeoutSeconds)
                || timeoutSeconds < 30
                || timeoutSeconds > 900))
        {
            error = "AUTOLIVE_DOUYIN_TIMEOUT_SEC 必须是 30～900 的整数。";
            return false;
        }

        if (!string.IsNullOrWhiteSpace(timeoutText))
        {
            timeout = TimeSpan.FromSeconds(int.Parse(timeoutText, System.Globalization.CultureInfo.InvariantCulture));
        }

        var qrRoot = string.IsNullOrWhiteSpace(temporaryDirectory)
            ? Path.GetTempPath()
            : temporaryDirectory.Trim();
        var qrOutput = Path.Combine(qrRoot, $"gpautolive-douyin-{Guid.NewGuid():N}.png");
        request = new WindowsDouyinProbeLaunchRequest(
            upstreamRoot,
            probeScript,
            condaExecutable,
            condaEnvironment,
            qrOutput,
            config,
            timeout);
        return true;
    }
}
