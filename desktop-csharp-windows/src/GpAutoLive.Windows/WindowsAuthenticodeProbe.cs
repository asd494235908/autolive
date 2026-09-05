using System.Runtime.InteropServices;

namespace GpAutoLive.Windows;

/// <summary>Windows Authenticode 只读探测结果。</summary>
public enum WindowsAuthenticodeProbeCode
{
    Valid,
    Unsigned,
    InvalidSignature,
    FileMissing,
    InvalidPath,
    NotWindows,
    ApiUnavailable,
    ProbeFailed,
}

/// <summary>签名探测快照；不返回证书主题、文件路径或原生错误正文。</summary>
public sealed record WindowsAuthenticodeProbeResult(
    WindowsAuthenticodeProbeCode Code)
{
    public bool IsValid => Code == WindowsAuthenticodeProbeCode.Valid;
}

/// <summary>
/// 使用 Windows Trust Provider 检查发布文件签名。
/// <para>
/// 探测不显示 UI、不联网刷新吊销状态、不修改文件；开发构建未签名时只返回
/// <see cref="WindowsAuthenticodeProbeCode.Unsigned"/>，不把它提升为发布可用。
/// </para>
/// </summary>
public static class WindowsAuthenticodeProbe
{
    private const int MaxPathCharacters = 32_000;
    private const uint WinTrustUiNone = 2;
    private const uint WinTrustRevokeNone = 0;
    private const uint WinTrustChoiceFile = 1;
    private const uint WinTrustStateActionIgnore = 0;
    private const uint WinTrustRevocationCheckNone = 0x00000010;
    private const uint WinTrustCacheOnlyUrlRetrieval = 0x00001000;
    private const uint TrustENoSignature = 0x800B0100;

    private static readonly Guid GenericVerifyV2 =
        new("00AAC56B-CD44-11D0-8CC2-00C04FC295EE");

    /// <summary>检查一个绝对、普通文件路径的 Authenticode 状态。</summary>
    public static WindowsAuthenticodeProbeResult Probe(string? filePath)
    {
        if (!OperatingSystem.IsWindows())
        {
            return new(WindowsAuthenticodeProbeCode.NotWindows);
        }

        if (!TryNormalizeFilePath(filePath, out var normalizedPath))
        {
            return new(WindowsAuthenticodeProbeCode.InvalidPath);
        }

        try
        {
            if (Directory.Exists(normalizedPath))
            {
                return new(WindowsAuthenticodeProbeCode.InvalidPath);
            }

            if (!File.Exists(normalizedPath))
            {
                return new(WindowsAuthenticodeProbeCode.FileMissing);
            }

            var attributes = File.GetAttributes(normalizedPath);
            if (attributes.HasFlag(FileAttributes.Directory)
                || attributes.HasFlag(FileAttributes.ReparsePoint))
            {
                return new(WindowsAuthenticodeProbeCode.InvalidPath);
            }

            return VerifyFile(normalizedPath);
        }
        catch (UnauthorizedAccessException)
        {
            return new(WindowsAuthenticodeProbeCode.ProbeFailed);
        }
        catch (IOException)
        {
            return new(WindowsAuthenticodeProbeCode.ProbeFailed);
        }
        catch (NotSupportedException)
        {
            return new(WindowsAuthenticodeProbeCode.ProbeFailed);
        }
    }

    private static WindowsAuthenticodeProbeResult VerifyFile(string path)
    {
        IntPtr filePathPointer = IntPtr.Zero;
        IntPtr fileInfoPointer = IntPtr.Zero;
        try
        {
            filePathPointer = Marshal.StringToCoTaskMemUni(path);
            var fileInfo = new WinTrustFileInfo
            {
                StructSize = (uint)Marshal.SizeOf<WinTrustFileInfo>(),
                FilePath = filePathPointer,
            };
            fileInfoPointer = Marshal.AllocHGlobal(Marshal.SizeOf<WinTrustFileInfo>());
            Marshal.StructureToPtr(fileInfo, fileInfoPointer, fDeleteOld: false);

            var trustData = new WinTrustData
            {
                StructSize = (uint)Marshal.SizeOf<WinTrustData>(),
                UiChoice = WinTrustUiNone,
                RevocationChecks = WinTrustRevokeNone,
                UnionChoice = WinTrustChoiceFile,
                FileInfo = fileInfoPointer,
                StateAction = WinTrustStateActionIgnore,
                ProviderFlags = WinTrustRevocationCheckNone | WinTrustCacheOnlyUrlRetrieval,
            };

            var actionIdentifier = GenericVerifyV2;
            var status = WinVerifyTrust(
                IntPtr.Zero,
                ref actionIdentifier,
                ref trustData);
            return status switch
            {
                0 => new(WindowsAuthenticodeProbeCode.Valid),
                TrustENoSignature => new(WindowsAuthenticodeProbeCode.Unsigned),
                _ => new(WindowsAuthenticodeProbeCode.InvalidSignature),
            };
        }
        catch (DllNotFoundException)
        {
            return new(WindowsAuthenticodeProbeCode.ApiUnavailable);
        }
        catch (EntryPointNotFoundException)
        {
            return new(WindowsAuthenticodeProbeCode.ApiUnavailable);
        }
        catch (BadImageFormatException)
        {
            return new(WindowsAuthenticodeProbeCode.ApiUnavailable);
        }
        catch (ExternalException)
        {
            return new(WindowsAuthenticodeProbeCode.ProbeFailed);
        }
        catch (ArgumentException)
        {
            return new(WindowsAuthenticodeProbeCode.ProbeFailed);
        }
        finally
        {
            if (fileInfoPointer != IntPtr.Zero)
            {
                Marshal.DestroyStructure<WinTrustFileInfo>(fileInfoPointer);
                Marshal.FreeHGlobal(fileInfoPointer);
            }

            if (filePathPointer != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(filePathPointer);
            }
        }
    }

    private static bool TryNormalizeFilePath(string? filePath, out string normalizedPath)
    {
        normalizedPath = string.Empty;
        if (string.IsNullOrWhiteSpace(filePath)
            || filePath.Length > MaxPathCharacters
            || filePath.Any(char.IsControl)
            || !Path.IsPathFullyQualified(filePath))
        {
            return false;
        }

        try
        {
            normalizedPath = Path.GetFullPath(filePath);
            return normalizedPath.Length <= MaxPathCharacters;
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (NotSupportedException)
        {
            return false;
        }
    }

    [DllImport("wintrust.dll", ExactSpelling = true, CharSet = CharSet.Unicode)]
    private static extern uint WinVerifyTrust(
        IntPtr windowHandle,
        ref Guid actionIdentifier,
        ref WinTrustData trustData);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct WinTrustFileInfo
    {
        public uint StructSize;
        public IntPtr FilePath;
        public IntPtr FileHandle;
        public IntPtr KnownSubject;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct WinTrustData
    {
        public uint StructSize;
        public IntPtr PolicyCallbackData;
        public IntPtr SipClientData;
        public uint UiChoice;
        public uint RevocationChecks;
        public uint UnionChoice;
        public IntPtr FileInfo;
        public uint StateAction;
        public IntPtr StateData;
        public IntPtr UrlReference;
        public uint ProviderFlags;
        public uint UiContext;
    }
}
