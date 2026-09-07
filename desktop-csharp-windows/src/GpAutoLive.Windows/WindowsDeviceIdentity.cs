using System.Security.Cryptography;
using System.Text;
using GpAutoLive.Contracts;
using GpAutoLive.Core.Security;
using GpAutoLive.Windows.Security;

namespace GpAutoLive.Windows;

/// <summary>
/// Windows 用户范围的稳定设备标识。只把随机标识存入 Credential Manager，
/// 不读取 MAC、硬盘序列号或用户目录文件列表。
/// </summary>
public static class WindowsDeviceIdentity
{
    public const string CredentialName = "control-plane-device-id";
    internal const string LegacyRustCredentialName = "device-id.autolive.desktop";

    private static readonly UTF8Encoding StrictUtf8 = new(false, true);
    private static readonly UnicodeEncoding StrictUtf16LittleEndian = new(false, false, true);

    public static string GetOrCreate(ISecretStore secretStore)
    {
        ArgumentNullException.ThrowIfNull(secretStore);
        return GetOrCreate(secretStore, CredentialManagerSecretStore.CreateLegacyRustDeviceIdentityReader());
    }

    internal static string GetOrCreate(ISecretStore secretStore, ISecretStore legacyRustSecretStore)
    {
        ArgumentNullException.ThrowIfNull(secretStore);
        ArgumentNullException.ThrowIfNull(legacyRustSecretStore);

        if (secretStore.TryGet(CredentialName, out var stored))
        {
            using (stored)
            {
                if (!TryReadDeviceId(stored, StrictUtf8, 1, out var existing))
                {
                    throw new InvalidDataException("Windows Credential Manager 中的设备标识无效。");
                }

                return existing;
            }
        }

        if (legacyRustSecretStore.TryGet(LegacyRustCredentialName, out var legacy))
        {
            using (legacy)
            {
                if (!TryReadDeviceId(legacy, StrictUtf16LittleEndian, sizeof(char), out var migrated))
                {
                    throw new InvalidDataException("Rust 桌面端保存的设备标识无效；为避免创建第二设备，C# 已停止自动迁移。");
                }

                StoreDeviceId(secretStore, migrated);
                return migrated;
            }
        }

        var created = $"desktop-{Guid.NewGuid():N}";
        StoreDeviceId(secretStore, created);
        return created;
    }

    private static bool TryReadDeviceId(
        SecretBuffer secret,
        Encoding encoding,
        int bytesPerCharacter,
        out string deviceId)
    {
        deviceId = string.Empty;
        if (secret.Length < AuthInputLimits.DeviceIdMinLength * bytesPerCharacter
            || secret.Length > AuthInputLimits.DeviceIdMaxLength * bytesPerCharacter
            || secret.Length % bytesPerCharacter != 0)
        {
            return false;
        }

        var bytes = new byte[secret.Length];
        try
        {
            secret.CopyTo(bytes);
            deviceId = encoding.GetString(bytes);
            return AuthContractValidation.TryValidateDeviceId(deviceId, out _);
        }
        catch (DecoderFallbackException)
        {
            deviceId = string.Empty;
            return false;
        }
        finally
        {
            CryptographicOperations.ZeroMemory(bytes);
        }
    }

    private static void StoreDeviceId(ISecretStore secretStore, string deviceId)
    {
        var bytes = Encoding.ASCII.GetBytes(deviceId);
        try
        {
            secretStore.Set(CredentialName, bytes);
        }
        finally
        {
            CryptographicOperations.ZeroMemory(bytes);
        }
    }
}
