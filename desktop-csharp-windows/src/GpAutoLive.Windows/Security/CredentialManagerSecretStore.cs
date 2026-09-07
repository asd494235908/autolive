using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using GpAutoLive.Core.Security;

namespace GpAutoLive.Windows.Security;

/// <summary>
/// 当前 Windows 用户的 Credential Manager 封装。凭据正文不进入文件、日志或异常消息。
/// </summary>
public sealed class CredentialManagerSecretStore : ISecretStore
{
    private const uint GenericCredentialType = 1;
    private const uint PersistLocalMachine = 2;
    private const uint ErrorNotFound = 1168;
    private const int MaxTargetNameLength = 256;
    private const int MaxSecretBytes = 64 * 1024;
    private const string DefaultTargetPrefix = "GpAutoLive.CSharp.Windows/";
    private const string LegacyRustDeviceIdTarget = "device-id.autolive.desktop";
    private readonly string _targetPrefix;

    public CredentialManagerSecretStore()
        : this(DefaultTargetPrefix)
    {
    }

    private CredentialManagerSecretStore(string targetPrefix)
    {
        ArgumentNullException.ThrowIfNull(targetPrefix);
        if (targetPrefix.Any(char.IsControl) || targetPrefix.Length >= MaxTargetNameLength)
        {
            throw new ArgumentException("凭据目标前缀格式无效。", nameof(targetPrefix));
        }

        _targetPrefix = targetPrefix;
    }

    internal static ISecretStore CreateLegacyRustDeviceIdentityReader() =>
        new ReadOnlyExactTargetStore(
            new CredentialManagerSecretStore(string.Empty),
            LegacyRustDeviceIdTarget);

    public void Set(string name, ReadOnlySpan<byte> secret)
    {
        var targetName = BuildTargetName(name);
        if (secret.Length == 0 || secret.Length > MaxSecretBytes)
        {
            throw new ArgumentOutOfRangeException(nameof(secret), "凭据长度不在允许范围内。");
        }

        var targetPointer = IntPtr.Zero;
        var blobPointer = IntPtr.Zero;
        var secretCopy = secret.ToArray();
        try
        {
            targetPointer = Marshal.StringToCoTaskMemUni(targetName);
            blobPointer = Marshal.AllocCoTaskMem(secretCopy.Length);
            Marshal.Copy(secretCopy, 0, blobPointer, secretCopy.Length);

            var credential = new NativeCredential
            {
                Type = GenericCredentialType,
                TargetName = targetPointer,
                CredentialBlob = blobPointer,
                CredentialBlobSize = (uint)secretCopy.Length,
                Persist = PersistLocalMachine
            };

            if (!CredWrite(ref credential, 0))
            {
                throw CreateWin32Exception("写入 Windows Credential Manager 失败。");
            }
        }
        finally
        {
            CryptographicOperations.ZeroMemory(secretCopy);
            FreeCoTaskMem(targetPointer);
            FreeCoTaskMem(blobPointer);
        }
    }

    public bool TryGet(string name, out SecretBuffer secret)
    {
        var targetName = BuildTargetName(name);
        if (!CredRead(targetName, GenericCredentialType, 0, out var credentialPointer))
        {
            var error = Marshal.GetLastWin32Error();
            if (error == ErrorNotFound)
            {
                secret = null!;
                return false;
            }

            throw CreateWin32Exception("读取 Windows Credential Manager 失败。", error);
        }

        try
        {
            var credential = Marshal.PtrToStructure<NativeCredential>(credentialPointer);
            if (credential.CredentialBlob == IntPtr.Zero
                || credential.CredentialBlobSize == 0
                || credential.CredentialBlobSize > MaxSecretBytes)
            {
                secret = null!;
                return false;
            }

            var bytes = new byte[(int)credential.CredentialBlobSize];
            Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
            secret = SecretBuffer.FromBytes(bytes);
            CryptographicOperations.ZeroMemory(bytes);
            return true;
        }
        finally
        {
            CredFree(credentialPointer);
        }
    }

    public bool Delete(string name)
    {
        var targetName = BuildTargetName(name);
        if (CredDelete(targetName, GenericCredentialType, 0))
        {
            return true;
        }

        var error = Marshal.GetLastWin32Error();
        if (error == ErrorNotFound)
        {
            return false;
        }

        throw CreateWin32Exception("删除 Windows Credential Manager 凭据失败。", error);
    }

    private string BuildTargetName(string name)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(name);
        var targetName = _targetPrefix + name;
        if (targetName.Length > MaxTargetNameLength
            || name.Any(character => char.IsControl(character) || character is '/' or '\\'))
        {
            throw new ArgumentException("凭据名称格式无效。", nameof(name));
        }

        return targetName;
    }

    private static Win32Exception CreateWin32Exception(string message, int? error = null)
    {
        var code = error ?? Marshal.GetLastWin32Error();
        return new Win32Exception(code, $"{message} 错误码：{code}。");
    }

    private static void FreeCoTaskMem(IntPtr pointer)
    {
        if (pointer != IntPtr.Zero)
        {
            Marshal.FreeCoTaskMem(pointer);
        }
    }

    private sealed class ReadOnlyExactTargetStore(ISecretStore inner, string allowedName) : ISecretStore
    {
        public void Set(string name, ReadOnlySpan<byte> secret) =>
            throw new NotSupportedException("旧 Rust 设备凭据只允许读取。");

        public bool TryGet(string name, out SecretBuffer secret)
        {
            if (!string.Equals(name, allowedName, StringComparison.Ordinal))
            {
                throw new ArgumentException("只允许读取固定的旧 Rust 设备凭据。", nameof(name));
            }

            return inner.TryGet(name, out secret);
        }

        public bool Delete(string name) =>
            throw new NotSupportedException("旧 Rust 设备凭据不允许删除。");
    }

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CredWrite(ref NativeCredential userCredential, uint flags);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CredRead(
        string target,
        uint type,
        uint reservedFlag,
        out IntPtr credential);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CredDelete(string target, uint type, uint flags);

    [DllImport("advapi32.dll")]
    private static extern void CredFree(IntPtr credential);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NativeCredential
    {
        public uint Flags;
        public uint Type;
        public IntPtr TargetName;
        public IntPtr Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint CredentialBlobSize;
        public IntPtr CredentialBlob;
        public uint Persist;
        public uint AttributeCount;
        public IntPtr Attributes;
        public IntPtr TargetAlias;
        public IntPtr UserName;
    }
}
