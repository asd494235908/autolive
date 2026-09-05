using GpAutoLive.Contracts;

namespace GpAutoLive.Core.Configuration;

/// <summary>插话配置的版本化 JSON 存储；目录快照和音频正文不写入此文件。</summary>
public sealed class InterludeAudioConfigStore
{
    /// <summary>当前插话配置 JSON schema 版本。</summary>
    public const int CurrentSchemaVersion = 1;
    /// <summary>插话配置 JSON 文件最大字节数。</summary>
    public const long MaxFileBytes = 64 * 1024;

    private readonly VersionedJsonStore<InterludeAudioConfig> _store;

    /// <summary>创建指定路径的插话配置存储。</summary>
    public InterludeAudioConfigStore(string path)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        _store = new VersionedJsonStore<InterludeAudioConfig>(path, CurrentSchemaVersion, MaxFileBytes);
    }

    /// <summary>文件不存在时返回参考端默认配置。</summary>
    public async Task<InterludeAudioConfig> ReadAsync(CancellationToken cancellationToken = default)
    {
        var document = await _store.ReadAsync(cancellationToken).ConfigureAwait(false);
        var config = document?.Data ?? InterludeAudioConfig.Default;
        if (!InterludeAudioRules.TryValidate(config, out var error))
        {
            throw new ConfigurationValidationException(error?.Message ?? "插话声音配置无效。");
        }

        return config;
    }

    /// <summary>校验后以原子方式保存。</summary>
    public Task WriteAsync(InterludeAudioConfig config, CancellationToken cancellationToken = default)
    {
        if (!InterludeAudioRules.TryValidate(config, out var error))
        {
            throw new ConfigurationValidationException(error?.Message ?? "插话声音配置无效。");
        }

        return _store.WriteAsync(config, cancellationToken);
    }
}
