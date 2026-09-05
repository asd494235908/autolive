using System.Runtime.InteropServices;
using System.Security.Cryptography;

namespace GpAutoLive.Windows.Security;

/// <summary>
/// 当前 Windows 用户范围的 DPAPI 边界。输出是受保护字节，不应写入普通配置而不再做访问控制。
/// </summary>
public static class DpapiSecretProtector
{
    public static byte[] Protect(ReadOnlySpan<byte> plaintext)
    {
        if (plaintext.Length == 0 || plaintext.Length > MaxPayloadBytes)
        {
            throw new ArgumentOutOfRangeException(nameof(plaintext), "DPAPI 输入长度不在允许范围内。");
        }

        return Invoke(plaintext, unprotect: false, "DPAPI 保护失败。");
    }

    public static byte[] Unprotect(ReadOnlySpan<byte> protectedData)
    {
        if (protectedData.Length == 0 || protectedData.Length > MaxPayloadBytes)
        {
            throw new ArgumentOutOfRangeException(nameof(protectedData), "DPAPI 输入长度不在允许范围内。");
        }

        return Invoke(protectedData, unprotect: true, "DPAPI 解保护失败。");
    }

    private const int MaxPayloadBytes = 64 * 1024;
    private const uint CryptProtectUiForbidden = 0x1;

    private static byte[] Invoke(
        ReadOnlySpan<byte> input,
        bool unprotect,
        string errorMessage)
    {
        var inputCopy = input.ToArray();
        var inputPointer = IntPtr.Zero;
        var outputBlob = default(DataBlob);
        try
        {
            inputPointer = Marshal.AllocCoTaskMem(inputCopy.Length);
            Marshal.Copy(inputCopy, 0, inputPointer, inputCopy.Length);
            var inputBlob = new DataBlob
            {
                Size = (uint)inputCopy.Length,
                Data = inputPointer
            };

            var succeeded = unprotect
                ? CryptUnprotectData(ref inputBlob, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, CryptProtectUiForbidden, ref outputBlob)
                : CryptProtectData(ref inputBlob, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, CryptProtectUiForbidden, ref outputBlob);
            if (!succeeded)
            {
                var error = Marshal.GetLastWin32Error();
                throw new CryptographicException($"{errorMessage} 错误码：{error}。");
            }

            if (outputBlob.Data == IntPtr.Zero || outputBlob.Size == 0 || outputBlob.Size > MaxPayloadBytes)
            {
                throw new CryptographicException($"{errorMessage} 输出无效。");
            }

            var output = new byte[(int)outputBlob.Size];
            Marshal.Copy(outputBlob.Data, output, 0, output.Length);
            return output;
        }
        finally
        {
            CryptographicOperations.ZeroMemory(inputCopy);
            if (inputPointer != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(inputPointer);
            }

            if (outputBlob.Data != IntPtr.Zero)
            {
                LocalFree(outputBlob.Data);
            }
        }
    }

    [DllImport("Crypt32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CryptProtectData(
        ref DataBlob dataIn,
        IntPtr description,
        IntPtr optionalEntropy,
        IntPtr reserved,
        IntPtr promptStruct,
        uint flags,
        ref DataBlob dataOut);

    [DllImport("Crypt32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CryptUnprotectData(
        ref DataBlob dataIn,
        IntPtr description,
        IntPtr optionalEntropy,
        IntPtr reserved,
        IntPtr promptStruct,
        uint flags,
        ref DataBlob dataOut);

    [DllImport("Kernel32.dll")]
    private static extern IntPtr LocalFree(IntPtr memory);

    [StructLayout(LayoutKind.Sequential)]
    private struct DataBlob
    {
        public uint Size;
        public IntPtr Data;
    }
}
