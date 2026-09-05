using System.Security.Cryptography;
using System.Text;
using GpAutoLive.Windows.Security;
using GpAutoLive.Core.Security;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class SecurityBoundaryTests
{
    [TestMethod]
    public void Credential_manager_round_trip_keeps_secret_out_of_files()
    {
        var name = $"test-{Guid.NewGuid():N}";
        var secretBytes = "test-secret-do-not-log"u8.ToArray();
        var configPath = Path.Combine(Path.GetTempPath(), "GpAutoLive.CSharp.Tests", $"{Guid.NewGuid():N}.ini");
        var store = new CredentialManagerSecretStore();

        try
        {
            store.Set(name, secretBytes);

            Assert.IsTrue(store.TryGet(name, out var loaded));
            using (loaded)
            {
                var copy = new byte[loaded.Length];
                loaded.CopyTo(copy);
                CollectionAssert.AreEqual(secretBytes, copy);
                CryptographicOperations.ZeroMemory(copy);
            }

            Assert.IsFalse(File.Exists(configPath));
        }
        finally
        {
            store.Delete(name);
            CryptographicOperations.ZeroMemory(secretBytes);
        }
    }

    [TestMethod]
    public void Dpapi_round_trip_and_secret_buffer_are_clearable()
    {
        var plaintext = "local-user-secret"u8.ToArray();
        var protectedBytes = DpapiSecretProtector.Protect(plaintext);
        Assert.AreNotEqual(Encoding.UTF8.GetString(plaintext), Encoding.UTF8.GetString(protectedBytes));

        var restored = DpapiSecretProtector.Unprotect(protectedBytes);
        CollectionAssert.AreEqual(plaintext, restored);

        using var buffer = SecretBuffer.FromBytes(restored);
        CryptographicOperations.ZeroMemory(plaintext);
        CryptographicOperations.ZeroMemory(protectedBytes);
        CryptographicOperations.ZeroMemory(restored);
    }
}
