using GpAutoLive.Contracts;

namespace GpAutoLive.Core.Configuration;

/// <summary>抖音 M1 非敏感本地配置的版本化 JSON 存储；不会保存登录凭据。</summary>
public sealed class DouyinLiveConfigStore
{
    /// <summary>当前抖音本地配置 schema 版本。</summary>
    public const int CurrentSchemaVersion = 1;
    /// <summary>抖音本地配置 JSON 最大字节数。</summary>
    public const long MaxFileBytes = 64 * 1024;

    private readonly VersionedJsonStore<DouyinLiveConfig> _store;

    /// <summary>创建指定路径的抖音配置存储。</summary>
    public DouyinLiveConfigStore(string path)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        _store = new VersionedJsonStore<DouyinLiveConfig>(path, CurrentSchemaVersion, MaxFileBytes);
    }

    /// <summary>读取并验证配置；文件不存在时返回安全默认配置。</summary>
    public async Task<DouyinLiveConfig> ReadAsync(CancellationToken cancellationToken = default)
    {
        var document = await _store.ReadAsync(cancellationToken).ConfigureAwait(false);
        var config = document?.Data ?? DouyinLiveConfig.Default;
        if (!DouyinLiveRules.TryNormalize(config, out var normalized, out var error))
        {
            throw new ConfigurationValidationException(error?.Message ?? "抖音 M1 配置无效。");
        }

        return normalized!;
    }

    /// <summary>校验后以版本化 JSON 原子写入配置。</summary>
    public Task WriteAsync(
        DouyinLiveConfig config,
        CancellationToken cancellationToken = default)
    {
        if (!DouyinLiveRules.TryNormalize(config, out var normalized, out var error))
        {
            throw new ConfigurationValidationException(error?.Message ?? "抖音 M1 配置无效。");
        }

        return _store.WriteAsync(normalized!, cancellationToken);
    }
}
