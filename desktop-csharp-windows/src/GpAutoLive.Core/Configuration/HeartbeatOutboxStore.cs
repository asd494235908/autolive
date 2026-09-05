using System.Text.Json.Serialization;
using GpAutoLive.Contracts;

namespace GpAutoLive.Core.Configuration;

/// <summary>
/// 单条待补发心跳。这里没有任何 Access/Refresh Token；身份字段只用于防止账号切换时
/// 把旧账号的运行摘要交给新会话。
/// </summary>
public sealed record HeartbeatOutboxEntry(
    [property: JsonPropertyName("user_id")] string UserId,
    [property: JsonPropertyName("device_id")] string DeviceId,
    [property: JsonPropertyName("idempotency_key")] string IdempotencyKey,
    [property: JsonPropertyName("request")] HeartbeatRequestDto Request,
    [property: JsonPropertyName("queued_at")] DateTimeOffset QueuedAt)
{
    /// <summary>用户/设备标识的最大长度。</summary>
    public const int MaxIdentityLength = AuthInputLimits.DeviceIdMaxLength;
    /// <summary>心跳幂等键的最大长度。</summary>
    public const int MaxIdempotencyKeyLength = AuthInputLimits.RequestIdMaxLength;
    /// <summary>待补发心跳的最大保留时间。</summary>
    public static readonly TimeSpan MaxAge = TimeSpan.FromHours(24);

    /// <summary>判断 outbox 是否属于当前账号和设备。</summary>
    public bool Matches(string userId, string deviceId) =>
        string.Equals(UserId, userId, StringComparison.Ordinal)
        && string.Equals(DeviceId, deviceId, StringComparison.Ordinal)
        && Request is not null
        && string.Equals(Request.DeviceId, deviceId, StringComparison.Ordinal);

    /// <summary>验证字段、请求和时间窗口。</summary>
    public bool IsValid(DateTimeOffset now)
    {
        return IsIdentity(UserId)
            && AuthContractValidation.TryValidateDeviceId(DeviceId, out _)
            && IsIdentity(IdempotencyKey, MaxIdempotencyKeyLength)
            && Request is not null
            && AuthContractValidation.TryValidateHeartbeat(Request, out _)
            && QueuedAt != default
            && QueuedAt <= now
            && now - QueuedAt <= MaxAge;
    }

    private static bool IsIdentity(string? value, int maximum = MaxIdentityLength) =>
        !string.IsNullOrWhiteSpace(value)
        && value.Length <= maximum
        && !value.Any(char.IsControl);
}

/// <summary>
/// 心跳 outbox 的单文件原子存储：最多一条、按账号/设备校验、损坏或过期项安全丢弃。
/// 它只保存可重放的心跳请求，不保存任何凭据。
/// </summary>
public sealed class HeartbeatOutboxStore
{
    /// <summary>outbox JSON 封套版本。</summary>
    public const int CurrentSchemaVersion = 1;
    /// <summary>outbox 文件最大字节数。</summary>
    public const long MaxFileBytes = 64 * 1024;

    private readonly string _path;
    private readonly VersionedJsonStore<HeartbeatOutboxEntry> _store;

    /// <summary>创建指定路径的单条心跳 outbox。</summary>
    public HeartbeatOutboxStore(string path)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        _path = Path.GetFullPath(path);
        _store = new VersionedJsonStore<HeartbeatOutboxEntry>(
            _path,
            CurrentSchemaVersion,
            MaxFileBytes);
    }

    /// <summary>读取匹配账号/设备且未过期的最新心跳。</summary>
    public async Task<HeartbeatOutboxEntry?> ReadAsync(
        string userId,
        string deviceId,
        DateTimeOffset now,
        CancellationToken cancellationToken = default)
    {
        if (!IsIdentity(userId)
            || !AuthContractValidation.TryValidateDeviceId(deviceId, out _))
        {
            return null;
        }

        VersionedJsonDocument<HeartbeatOutboxEntry>? document;
        try
        {
            document = await _store.ReadAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ConfigurationValidationException)
        {
            TryDelete();
            return null;
        }
        catch (IOException)
        {
            return null;
        }
        catch (UnauthorizedAccessException)
        {
            return null;
        }

        var entry = document?.Data;
        if (entry is null || !entry.Matches(userId, deviceId) || !entry.IsValid(now))
        {
            if (entry is not null)
            {
                TryDelete();
            }

            return null;
        }

        return entry;
    }

    /// <summary>原子替换为一条最新心跳。</summary>
    public Task SaveLatestAsync(
        HeartbeatOutboxEntry entry,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(entry);
        if (!entry.IsValid(entry.QueuedAt))
        {
            throw new ConfigurationValidationException("待补发心跳内容无效。");
        }

        return _store.WriteAsync(entry, cancellationToken);
    }

    /// <summary>按账号/设备和可选幂等键安全清除 outbox。</summary>
    public async Task ClearAsync(
        string userId,
        string deviceId,
        string? idempotencyKey = null,
        CancellationToken cancellationToken = default,
        DateTimeOffset? now = null)
    {
        var current = await ReadAsync(userId, deviceId, now ?? DateTimeOffset.UtcNow, cancellationToken).ConfigureAwait(false);
        if (current is null || (idempotencyKey is not null
            && !string.Equals(current.IdempotencyKey, idempotencyKey, StringComparison.Ordinal)))
        {
            return;
        }

        TryDelete();
    }

    private static bool IsIdentity(string? value) =>
        !string.IsNullOrWhiteSpace(value)
        && value.Length <= HeartbeatOutboxEntry.MaxIdentityLength
        && !value.Any(char.IsControl);

    private void TryDelete()
    {
        try
        {
            File.Delete(_path);
        }
        catch (IOException)
        {
            // outbox 是可丢弃缓存；删除失败不能阻断当前心跳。
        }
        catch (UnauthorizedAccessException)
        {
            // 同上，不把本地路径或文件内容带入错误。
        }
    }
}
