using System.Security.Cryptography;
using System.Text;
using GpAutoLive.Contracts;
using GpAutoLive.Core.Security;

namespace GpAutoLive.Windows;

/// <summary>
/// Windows 用户范围的稳定设备标识。只把随机标识存入 Credential Manager，
/// 不读取 MAC、硬盘序列号或用户目录文件列表。
/// </summary>
public static class WindowsDeviceIdentity
{
    public const string CredentialName = "control-plane-device-id";

    private static readonly UTF8Encoding StrictUtf8 = new(false, true);

    public static string GetOrCreate(ISecretStore secretStore)
    {
        ArgumentNullException.ThrowIfNull(secretStore);

        if (secretStore.TryGet(CredentialName, out var stored))
        {
            using (stored)
            {
                var bytes = new byte[stored.Length];
                try
                {
                    stored.CopyTo(bytes);
                    var value = StrictUtf8.GetString(bytes);
                    if (!AuthContractValidation.TryValidateDeviceId(value, out _))
                    {
                        throw new InvalidDataException("Windows Credential Manager 中的设备标识无效。");
                    }

                    return value;
                }
                finally
                {
                    CryptographicOperations.ZeroMemory(bytes);
                }
            }
        }

        var created = $"desktop-{Guid.NewGuid():N}";
        var createdBytes = Encoding.ASCII.GetBytes(created);
        try
        {
            secretStore.Set(CredentialName, createdBytes);
            return created;
        }
        finally
        {
            CryptographicOperations.ZeroMemory(createdBytes);
        }
    }
}
