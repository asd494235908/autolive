using System.Collections.Immutable;
using System.Buffers.Binary;
using System.Security.Cryptography;
using GpAutoLive.Contracts;

namespace GpAutoLive.Windows;

/// <summary>AkVirtualCamera sidecar 启动请求；会话令牌只在本次内存生命周期存在。</summary>
public sealed record WindowsVirtualCameraSidecarLaunchRequest(
    string SidecarExecutablePath,
    byte[] SessionToken,
    VirtualCameraConfig? Config,
    TimeSpan StartupTimeout);

/// <summary>已经通过路径、规格和令牌校验的 sidecar 启动计划。</summary>
public sealed record WindowsVirtualCameraSidecarLaunchPlan(
    string ExecutablePath,
    string WorkingDirectory,
    ImmutableArray<string> Arguments,
    VirtualCameraConfig Config,
    TimeSpan StartupTimeout)
{
    /// <summary>受控 sidecar 进程读取 stdin 令牌时使用的内存值。</summary>
    internal byte[] SessionToken { get; init; } = [];

    /// <summary>由固定令牌派生的本机管道名称；不进入普通配置或 UI 快照。</summary>
    internal string PipeName { get; init; } = string.Empty;
}

/// <summary>构造 AkVirtualCamera sidecar 的最小安全启动计划；不会启动进程。</summary>
public static class WindowsVirtualCameraSidecarLaunchPlanBuilder
{
    /// <summary>固定的 x64 GPL sidecar 文件名。</summary>
    public const string SidecarFileName = "akvirtualcamera-sidecar-x64.exe";
    /// <summary>默认等待 sidecar 创建并接受管道连接的时间。</summary>
    public static readonly TimeSpan DefaultStartupTimeout = TimeSpan.FromSeconds(5);
    /// <summary>允许的最短启动等待时间。</summary>
    public static readonly TimeSpan MinStartupTimeout = TimeSpan.FromMilliseconds(100);
    /// <summary>允许的最长启动等待时间。</summary>
    public static readonly TimeSpan MaxStartupTimeout = TimeSpan.FromSeconds(30);
    /// <summary>sidecar 文件大小上限，避免把任意大文件当作组件加载。</summary>
    public const long MaxSidecarBytes = 64L * 1024 * 1024;
    /// <summary>sidecar 状态行上限，供后续受管宿主复用同一安全边界。</summary>
    public const int MaxStatusLineBytes = 4 * 1024;
    /// <summary>构造隐藏、无 Shell、stdin 令牌传递的不可变计划。</summary>
    public static bool TryCreate(
        WindowsVirtualCameraSidecarLaunchRequest? request,
        out WindowsVirtualCameraSidecarLaunchPlan? plan,
        out string? error)
    {
        plan = null;
        error = null;
        if (!OperatingSystem.IsWindows())
        {
            error = "AkVirtualCamera sidecar 仅支持 Windows。";
            return false;
        }

        if (request is null
            || !TryValidateTimeout(request.StartupTimeout)
            || request.Config is null
            || !request.Config.TryValidateFixedOutput(out _))
        {
            error = "AkVirtualCamera sidecar 启动参数或固定输出配置无效。";
            return false;
        }

        if (!TryResolveValidatedSidecar(request.SidecarExecutablePath, out var sidecarPath))
        {
            error = "AkVirtualCamera sidecar 必须是固定名称、绝对路径下的普通 x64 文件。";
            return false;
        }

        if (!WindowsVirtualCameraSidecarProtocol.TryCreatePipeName(
                request.SessionToken,
                out var pipeName,
                out _)
            || pipeName is null)
        {
            error = "AkVirtualCamera sidecar 会话令牌无效。";
            return false;
        }

        var workingDirectory = Path.GetDirectoryName(sidecarPath);
        if (workingDirectory is null)
        {
            error = "AkVirtualCamera sidecar 工作目录无效。";
            return false;
        }

        plan = new WindowsVirtualCameraSidecarLaunchPlan(
            sidecarPath,
            workingDirectory,
            ImmutableArray.Create("--session-token-stdin"),
            request.Config,
            request.StartupTimeout)
        {
            SessionToken = request.SessionToken.ToArray(),
            PipeName = pipeName,
        };
        return true;
    }

    /// <summary>生成非零随机会话令牌；令牌不得写入命令行、环境变量或普通配置。</summary>
    public static byte[] CreateSessionToken()
    {
        var token = RandomNumberGenerator.GetBytes(16);
        if (token.All(static value => value == 0))
        {
            token[0] = 1;
        }

        return token;
    }

    private static bool TryValidateTimeout(TimeSpan value) =>
        value >= MinStartupTimeout && value <= MaxStartupTimeout;

    internal static bool TryResolveValidatedSidecar(string? value, out string path)
    {
        path = string.Empty;
        if (string.IsNullOrWhiteSpace(value) || value.Any(char.IsControl))
        {
            return false;
        }

        try
        {
            path = Path.GetFullPath(value.Trim());
            if (!Path.IsPathFullyQualified(path)
                || !string.Equals(Path.GetFileName(path), SidecarFileName, StringComparison.Ordinal)
                || !File.Exists(path))
            {
                return false;
            }

            var attributes = File.GetAttributes(path);
            if ((attributes & FileAttributes.ReparsePoint) != 0)
            {
                return false;
            }

            var directory = Path.GetDirectoryName(path);
            if (directory is null
                || !Directory.Exists(directory)
                || (File.GetAttributes(directory) & FileAttributes.ReparsePoint) != 0)
            {
                return false;
            }

            var info = new FileInfo(path);
            return info.Length is > 0 and <= MaxSidecarBytes
                && TryValidateX64PortableExecutable(path, info.Length);
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (NotSupportedException)
        {
            return false;
        }
        catch (IOException)
        {
            return false;
        }
        catch (UnauthorizedAccessException)
        {
            return false;
        }
    }

    private static bool TryValidateX64PortableExecutable(string path, long fileLength)
    {
        const int DosHeaderBytes = 64;
        const int PeSignatureAndCoffBytes = 24;
        const int Pe32PlusMagicBytes = 2;
        const int PeOffsetField = 0x3c;
        const int MaximumHeaderOffset = 1 * 1024 * 1024;
        const ushort Amd64Machine = 0x8664;
        const ushort Pe32PlusMagic = 0x20b;

        if (fileLength < DosHeaderBytes)
        {
            return false;
        }

        try
        {
            using var stream = new FileStream(
                path,
                FileMode.Open,
                FileAccess.Read,
                FileShare.Read,
                bufferSize: 512,
                FileOptions.SequentialScan);
            Span<byte> dosHeader = stackalloc byte[DosHeaderBytes];
            stream.ReadExactly(dosHeader);
            if (dosHeader[0] != (byte)'M' || dosHeader[1] != (byte)'Z')
            {
                return false;
            }

            var peOffset = BinaryPrimitives.ReadInt32LittleEndian(dosHeader[PeOffsetField..]);
            if (peOffset < DosHeaderBytes
                || peOffset > MaximumHeaderOffset
                || (long)peOffset + PeSignatureAndCoffBytes + Pe32PlusMagicBytes > fileLength)
            {
                return false;
            }

            stream.Position = peOffset;
            Span<byte> peHeader = stackalloc byte[PeSignatureAndCoffBytes + Pe32PlusMagicBytes];
            stream.ReadExactly(peHeader);
            if (peHeader[0] != (byte)'P'
                || peHeader[1] != (byte)'E'
                || peHeader[2] != 0
                || peHeader[3] != 0)
            {
                return false;
            }

            var machine = BinaryPrimitives.ReadUInt16LittleEndian(peHeader[4..]);
            var optionalHeaderBytes = BinaryPrimitives.ReadUInt16LittleEndian(peHeader[20..]);
            var optionalMagic = BinaryPrimitives.ReadUInt16LittleEndian(peHeader[24..]);
            return machine == Amd64Machine
                && optionalHeaderBytes >= Pe32PlusMagicBytes
                && optionalMagic == Pe32PlusMagic;
        }
        catch (EndOfStreamException)
        {
            return false;
        }
        catch (IOException)
        {
            return false;
        }
        catch (UnauthorizedAccessException)
        {
            return false;
        }
    }
}
