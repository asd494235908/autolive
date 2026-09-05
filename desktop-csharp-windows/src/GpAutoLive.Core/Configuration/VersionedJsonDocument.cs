using System.Text.Json;
using System.Text.Json.Serialization;
using GpAutoLive.Contracts;

namespace GpAutoLive.Core.Configuration;

/// <summary>结构化配置的版本封套；INI 偏好不复用此格式。</summary>
public sealed record VersionedJsonDocument<T>
    where T : class
{
    /// <summary>JSON 文档格式版本。</summary>
    [JsonPropertyName("schema_version")]
    public int SchemaVersion { get; init; }

    /// <summary>版本封套中的业务数据。</summary>
    [JsonPropertyName("data")]
    public T? Data { get; init; }
}

/// <summary>版本化 JSON 的有界、严格、原子读写器。</summary>
public sealed class VersionedJsonStore<T>
    where T : class
{
    /// <summary>JSON 文件默认最大字节数。</summary>
    public const long DefaultMaxFileBytes = 4 * 1024 * 1024;

    private readonly string _path;
    private readonly int _currentSchemaVersion;
    private readonly long _maxFileBytes;
    private readonly JsonSerializerOptions _jsonOptions;

    /// <summary>创建版本化 JSON 原子存储。</summary>
    public VersionedJsonStore(
        string path,
        int currentSchemaVersion,
        long maxFileBytes = DefaultMaxFileBytes,
        JsonSerializerOptions? jsonOptions = null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        if (currentSchemaVersion <= 0)
        {
            throw new ArgumentOutOfRangeException(nameof(currentSchemaVersion));
        }

        if (maxFileBytes <= 0 || maxFileBytes > 64 * 1024 * 1024)
        {
            throw new ArgumentOutOfRangeException(nameof(maxFileBytes));
        }

        _path = Path.GetFullPath(path);
        _currentSchemaVersion = currentSchemaVersion;
        _maxFileBytes = maxFileBytes;
        _jsonOptions = jsonOptions ?? ContractJson.CreateOptions();
    }

    /// <summary>异步读取并验证版本化文档；文件不存在时返回 null。</summary>
    public async Task<VersionedJsonDocument<T>?> ReadAsync(CancellationToken cancellationToken = default)
    {
        if (!File.Exists(_path))
        {
            return null;
        }

        string text;
        try
        {
            text = await AtomicFile.ReadTextAsync(_path, _maxFileBytes, cancellationToken).ConfigureAwait(false);
        }
        catch (FileNotFoundException)
        {
            return null;
        }

        VersionedJsonDocument<T>? document;
        try
        {
            document = JsonSerializer.Deserialize<VersionedJsonDocument<T>>(text, _jsonOptions);
        }
        catch (JsonException exception)
        {
            throw new ConfigurationValidationException("JSON 配置格式无效。", exception);
        }

        if (document is null || document.Data is null)
        {
            throw new ConfigurationValidationException("JSON 配置内容不能为空。");
        }

        if (document.SchemaVersion != _currentSchemaVersion)
        {
            throw new ConfigurationValidationException("JSON 配置版本不受支持。");
        }

        return document;
    }

    /// <summary>异步序列化并原子写入版本化文档。</summary>
    public Task WriteAsync(
        T data,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(data);
        var document = new VersionedJsonDocument<T>
        {
            SchemaVersion = _currentSchemaVersion,
            Data = data
        };

        byte[] bytes;
        try
        {
            bytes = JsonSerializer.SerializeToUtf8Bytes(document, _jsonOptions);
        }
        catch (JsonException exception)
        {
            throw new ConfigurationValidationException("JSON 配置无法序列化。", exception);
        }

        if (bytes.LongLength > _maxFileBytes)
        {
            throw new ConfigurationValidationException("JSON 配置超过大小上限。");
        }

        SensitivePropertyGuard.EnsureAllowed(bytes);

        return AtomicFile.WriteTextAsync(_path, bytes, cancellationToken);
    }
}

/// <summary>阻止敏感字段进入普通 JSON 配置，即使调用方传入了错误 DTO。</summary>
internal static class SensitivePropertyGuard
{
    public static void EnsureAllowed(ReadOnlyMemory<byte> json)
    {
        try
        {
            using var document = JsonDocument.Parse(json);
            Visit(document.RootElement);
        }
        catch (JsonException exception)
        {
            throw new ConfigurationValidationException("JSON 配置内容无效。", exception);
        }
    }

    private static void Visit(JsonElement element)
    {
        if (element.ValueKind == JsonValueKind.Object)
        {
            foreach (var property in element.EnumerateObject())
            {
                if (IsSensitiveName(property.Name))
                {
                    throw new ConfigurationValidationException("JSON 配置包含禁止的敏感字段。");
                }

                Visit(property.Value);
            }

            return;
        }

        if (element.ValueKind == JsonValueKind.Array)
        {
            foreach (var item in element.EnumerateArray())
            {
                Visit(item);
            }
        }
    }

    private static bool IsSensitiveName(string name)
    {
        return name.Contains("password", StringComparison.OrdinalIgnoreCase)
            || name.Contains("token", StringComparison.OrdinalIgnoreCase)
            || name.Contains("secret", StringComparison.OrdinalIgnoreCase)
            || name.Contains("cookie", StringComparison.OrdinalIgnoreCase)
            || name.Contains("credential", StringComparison.OrdinalIgnoreCase)
            || name.Contains("api_key", StringComparison.OrdinalIgnoreCase)
            || name.Contains("authorization", StringComparison.OrdinalIgnoreCase)
            || name.Contains("rtmp_url", StringComparison.OrdinalIgnoreCase)
            || name.Contains("target_url", StringComparison.OrdinalIgnoreCase);
    }
}
