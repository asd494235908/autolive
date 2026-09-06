using System.Security.Cryptography;
using System.Text;
using GpAutoLive.Contracts;
using GpAutoLive.Core.Security;

namespace GpAutoLive.Windows;

/// <summary>在 Credential Manager 中保存少量待撤销 Refresh Token；每个槽位独立且有固定上限。</summary>
internal sealed class PendingLogoutTokenStore(ISecretStore secretStore)
{
    internal const int MaxPendingTokens = 8;

    public bool Enqueue(string refreshToken)
    {
        if (!AuthContractValidation.TryValidateRefreshToken(refreshToken, out _))
        {
            return false;
        }

        string? firstEmpty = null;
        for (var index = 0; index < MaxPendingTokens; index++)
        {
            var name = CredentialName(index);
            if (!secretStore.TryGet(name, out var secret))
            {
                firstEmpty ??= name;
                continue;
            }

            using (secret)
            {
                if (!TryReadToken(secret, out var existing))
                {
                    secretStore.Delete(name);
                    firstEmpty ??= name;
                    continue;
                }

                if (string.Equals(existing, refreshToken, StringComparison.Ordinal))
                {
                    return true;
                }
            }
        }

        if (firstEmpty is null)
        {
            return false;
        }

        var bytes = Encoding.UTF8.GetBytes(refreshToken);
        try
        {
            secretStore.Set(firstEmpty, bytes);
            return true;
        }
        finally
        {
            CryptographicOperations.ZeroMemory(bytes);
        }
    }

    public bool TryGetFirst(out string credentialName, out string refreshToken)
    {
        for (var index = 0; index < MaxPendingTokens; index++)
        {
            var name = CredentialName(index);
            if (!secretStore.TryGet(name, out var secret))
            {
                continue;
            }

            using (secret)
            {
                if (TryReadToken(secret, out refreshToken))
                {
                    credentialName = name;
                    return true;
                }
            }

            secretStore.Delete(name);
        }

        credentialName = string.Empty;
        refreshToken = string.Empty;
        return false;
    }

    public void Remove(string credentialName) => secretStore.Delete(credentialName);

    private static string CredentialName(int index) =>
        index == 0
            ? ControlPlaneAuthCoordinator.PendingLogoutCredentialName
            : $"{ControlPlaneAuthCoordinator.PendingLogoutCredentialName}-{index + 1}";

    private static bool TryReadToken(SecretBuffer secret, out string token)
    {
        token = string.Empty;
        if (secret.Length is <= 0 or > AuthInputLimits.TokenMaxLength)
        {
            return false;
        }

        var bytes = new byte[secret.Length];
        try
        {
            secret.CopyTo(bytes);
            token = new UTF8Encoding(false, true).GetString(bytes);
            return AuthContractValidation.TryValidateRefreshToken(token, out _);
        }
        catch (DecoderFallbackException)
        {
            token = string.Empty;
            return false;
        }
        finally
        {
            CryptographicOperations.ZeroMemory(bytes);
        }
    }
}
