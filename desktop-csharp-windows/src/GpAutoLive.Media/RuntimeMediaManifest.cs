using System.Collections.Immutable;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using GpAutoLive.Contracts;
using GpAutoLive.Core.Processes;

namespace GpAutoLive.Media;

/// <summary>
/// 安装目录中的媒体运行资源清单。清单位于
/// <c>runtime/media/&lt;runtime_version&gt;/manifest.json</c>，资源位于同版本的
/// <c>bin</c> 子目录；用户数据目录不参与资源解析。
/// </summary>
public sealed record RuntimeMediaManifestDto
{
    [JsonPropertyName("schema_version")]
    public int SchemaVersion { get; init; }

    [JsonPropertyName("runtime_version")]
    public string RuntimeVersion { get; init; } = string.Empty;

    [JsonPropertyName("platform")]
    public string Platform { get; init; } = string.Empty;

    [JsonPropertyName("architecture")]
    public string Architecture { get; init; } = string.Empty;

    [JsonPropertyName("resources")]
    public ImmutableArray<RuntimeMediaResourceDto> Resources { get; init; }
}

/// <summary>一个外置媒体资源的相对文件名、声明大小和 SHA-256。</summary>
public sealed record RuntimeMediaResourceDto
{
    [JsonPropertyName("name")]
    public string Name { get; init; } = string.Empty;

    [JsonPropertyName("relative_path")]
    public string RelativePath { get; init; } = string.Empty;

    [JsonPropertyName("size_bytes")]
    public long SizeBytes { get; init; }

    [JsonPropertyName("sha256")]
    public string Sha256 { get; init; } = string.Empty;
}

/// <summary>运行资源解析的稳定、脱敏错误分类。</summary>
public enum RuntimeResourceFailureCode
{
    EmptyManifest,
    ManifestTooLarge,
    InvalidManifestJson,
    UnknownField,
    DuplicateField,
    MissingField,
    UnsupportedSchemaVersion,
    InvalidRuntimeVersion,
    InvalidPlatform,
    InvalidArchitecture,
    EmptyResourceList,
    TooManyResources,
    DuplicateResource,
    UnknownResourceName,
    InvalidResourcePath,
    InvalidResourceSize,
    ResourceTotalTooLarge,
    InvalidSha256,
    InvalidInstallationDirectory,
    InstallationDirectoryMissing,
    ResourceDirectoryMissing,
    ManifestMissing,
    ResourceMissing,
    ResourceIsDirectory,
    ReparsePointNotAllowed,
    ResourceSizeMismatch,
    ResourceHashMismatch,
    ResourceUnavailable,
    MissingFfprobe,
    OperationCancelled
}

/// <summary>不会包含路径、文件名、哈希正文或底层异常文本。</summary>
public sealed record RuntimeResourceError(
    RuntimeResourceFailureCode Code,
    string Message,
    bool Retryable);

/// <summary>严格解析后的清单结果；失败时不会保留部分 DTO。</summary>
public sealed record RuntimeMediaManifestParseResult(
    RuntimeMediaManifestDto? Manifest,
    RuntimeResourceError? Error)
{
    public bool IsSuccess => Manifest is not null && Error is null;

    public static RuntimeMediaManifestParseResult Succeeded(RuntimeMediaManifestDto manifest) =>
        new(manifest, null);

    public static RuntimeMediaManifestParseResult Failed(RuntimeResourceError error) =>
        new(null, error);
}

/// <summary>单个已验证资源的不可变快照。</summary>
public sealed record VerifiedRuntimeResource(
    string Name,
    string AbsolutePath,
    long SizeBytes,
    string Sha256);

/// <summary>
/// 一次性完成清单内所有资源校验后的原子运行时快照。
/// 不暴露未验证路径；创建失败时整个快照为空。
/// </summary>
public sealed class VerifiedMediaRuntime
{
    private readonly ImmutableDictionary<string, VerifiedRuntimeResource> _resources;

    internal VerifiedMediaRuntime(
        string installationDirectory,
        string versionDirectory,
        string binDirectory,
        string runtimeVersion,
        ImmutableArray<VerifiedRuntimeResource> resources)
    {
        InstallationDirectory = installationDirectory;
        VersionDirectory = versionDirectory;
        BinDirectory = binDirectory;
        RuntimeVersion = runtimeVersion;
        Resources = resources;
        var builder = ImmutableDictionary.CreateBuilder<string, VerifiedRuntimeResource>(StringComparer.OrdinalIgnoreCase);
        foreach (var resource in resources)
        {
            builder.Add(resource.Name, resource);
        }

        _resources = builder.ToImmutable();
    }

    public string InstallationDirectory { get; }

    public string VersionDirectory { get; }

    public string BinDirectory { get; }

    public string RuntimeVersion { get; }

    public ImmutableArray<VerifiedRuntimeResource> Resources { get; }

    /// <summary>仅从本次已验证快照中取得固定白名单资源。</summary>
    public bool TryGetResource(string name, out VerifiedRuntimeResource? resource)
    {
        resource = null;
        if (!RuntimeMediaManifestBoundary.IsAllowedResourceName(name))
        {
            return false;
        }

        return _resources.TryGetValue(name, out resource);
    }

    /// <summary>
    /// 将 FFprobe 探测器绑定到已完成哈希校验的路径；该方法只创建对象，不启动进程。
    /// </summary>
    public bool TryCreateFfprobeProbe(
        IExternalProcessRunner processRunner,
        out FfprobeMediaProbe? probe,
        out RuntimeResourceError? error,
        FfprobeLimits? limits = null)
    {
        ArgumentNullException.ThrowIfNull(processRunner);
        probe = null;
        error = null;
        if (!TryGetResource("ffprobe.exe", out var resource) || resource is null)
        {
            error = RuntimeMediaManifestBoundary.Error(
                RuntimeResourceFailureCode.MissingFfprobe,
                "已验证的媒体运行资源缺少 FFprobe。",
                retryable: false);
            return false;
        }

        probe = new FfprobeMediaProbe(resource.AbsolutePath, processRunner, limits);
        return true;
    }
}

/// <summary>资源校验结果；只有所有清单项通过后才返回 Runtime。</summary>
public sealed record RuntimeMediaVerificationResult(
    VerifiedMediaRuntime? Runtime,
    RuntimeResourceError? Error)
{
    public bool IsSuccess => Runtime is not null && Error is null;

    public static RuntimeMediaVerificationResult Succeeded(VerifiedMediaRuntime runtime) =>
        new(runtime, null);

    public static RuntimeMediaVerificationResult Failed(RuntimeResourceError error) =>
        new(null, error);
}

/// <summary>
/// Windows x64 外置媒体资源清单解析和完整性校验入口。
/// </summary>
public static class RuntimeMediaManifestBoundary
{
    public const int CurrentSchemaVersion = 1;
    public const long MaxManifestBytes = 1 * 1024 * 1024;
    public const int MaxResourceCount = 64;
    public const long MaxResourceSizeBytes = 512L * 1024 * 1024;
    public const long MaxTotalResourceBytes = 2L * 1024 * 1024 * 1024;
    public const int MaxRuntimeVersionCharacters = 64;
    public const int MaxInstallationPathCharacters = 32_000;

    private const string ExpectedPlatform = "windows";
    private const string ExpectedArchitecture = "x64";
    private const string BinPrefix = "bin/";
    private const string ManifestFileName = "manifest.json";

    private static readonly ImmutableHashSet<string> AllowedResourceNames =
        ImmutableHashSet.Create(
            StringComparer.OrdinalIgnoreCase,
            "ffprobe.exe",
            "ffmpeg.exe",
            "mpv.exe",
            "portaudio_x64.dll",
            "d3dcompiler_43.dll",
            "gpu83.hook",
            "avcodec-61.dll",
            "avdevice-61.dll",
            "avfilter-10.dll",
            "avformat-61.dll",
            "avutil-59.dll",
            "swresample-5.dll",
            "swscale-8.dll",
            "libmpv-2.dll");

    /// <summary>用于测试/发布清单生成的固定资源名视图。</summary>
    public static IReadOnlySet<string> ResourceNameAllowList => AllowedResourceNames;

    /// <summary>以严格 JSON 选项解析清单，不访问磁盘。</summary>
    public static RuntimeMediaManifestParseResult Parse(string? manifestJson)
    {
        if (string.IsNullOrWhiteSpace(manifestJson))
        {
            return RuntimeMediaManifestParseResult.Failed(Error(
                RuntimeResourceFailureCode.EmptyManifest,
                "媒体运行资源清单不能为空。",
                retryable: false));
        }

        if (Encoding.UTF8.GetByteCount(manifestJson) > MaxManifestBytes)
        {
            return RuntimeMediaManifestParseResult.Failed(Error(
                RuntimeResourceFailureCode.ManifestTooLarge,
                "媒体运行资源清单超过大小限制。",
                retryable: false));
        }

        try
        {
            using var document = JsonDocument.Parse(manifestJson);
            var shapeError = ValidateJsonShape(document.RootElement);
            if (shapeError is not null)
            {
                return RuntimeMediaManifestParseResult.Failed(shapeError);
            }

            var manifest = JsonSerializer.Deserialize<RuntimeMediaManifestDto>(
                manifestJson,
                ContractJson.CreateOptions());
            if (manifest is null)
            {
                return RuntimeMediaManifestParseResult.Failed(Error(
                    RuntimeResourceFailureCode.InvalidManifestJson,
                    "媒体运行资源清单格式无效。",
                    retryable: false));
            }

            var validationError = ValidateManifest(manifest);
            return validationError is null
                ? RuntimeMediaManifestParseResult.Succeeded(manifest)
                : RuntimeMediaManifestParseResult.Failed(validationError);
        }
        catch (JsonException)
        {
            return RuntimeMediaManifestParseResult.Failed(Error(
                RuntimeResourceFailureCode.InvalidManifestJson,
                "媒体运行资源清单格式无效。",
                retryable: false));
        }
        catch (NotSupportedException)
        {
            return RuntimeMediaManifestParseResult.Failed(Error(
                RuntimeResourceFailureCode.InvalidManifestJson,
                "媒体运行资源清单格式无效。",
                retryable: false));
        }
        catch (OverflowException)
        {
            return RuntimeMediaManifestParseResult.Failed(Error(
                RuntimeResourceFailureCode.InvalidManifestJson,
                "媒体运行资源清单数值无效。",
                retryable: false));
        }
    }

    /// <summary>解析并验证安装目录下固定版本的 manifest.json。</summary>
    public static async Task<RuntimeMediaVerificationResult> LoadAndVerifyAsync(
        string? installationDirectory,
        string? runtimeVersion,
        CancellationToken cancellationToken = default)
    {
        var directoryError = ValidateInstallationDirectory(installationDirectory, requireExisting: true, out var installDirectory);
        if (directoryError is not null)
        {
            return RuntimeMediaVerificationResult.Failed(directoryError);
        }

        var versionError = ValidateRuntimeVersion(runtimeVersion, out var normalizedVersion);
        if (versionError is not null)
        {
            return RuntimeMediaVerificationResult.Failed(versionError);
        }

        string versionDirectory;
        string manifestPath;
        try
        {
            versionDirectory = Path.Combine(installDirectory, "runtime", "media", normalizedVersion);
            manifestPath = Path.Combine(versionDirectory, ManifestFileName);
        }
        catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.InvalidInstallationDirectory,
                "安装目录无效。",
                retryable: false));
        }

        if (!IsWithinDirectory(manifestPath, installDirectory))
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.InvalidInstallationDirectory,
                "媒体运行资源目录不在安装目录内。",
                retryable: false));
        }

        if (!File.Exists(manifestPath))
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.ManifestMissing,
                "媒体运行资源清单不存在。",
                retryable: true));
        }

        if (HasReparsePoint(manifestPath))
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.ReparsePointNotAllowed,
                "媒体运行资源不能通过重解析点加载。",
                retryable: false));
        }

        string manifestJson;
        try
        {
            var info = new FileInfo(manifestPath);
            if (info.Length > MaxManifestBytes)
            {
                return RuntimeMediaVerificationResult.Failed(Error(
                    RuntimeResourceFailureCode.ManifestTooLarge,
                    "媒体运行资源清单超过大小限制。",
                    retryable: false));
            }

            await using var stream = new FileStream(
                manifestPath,
                FileMode.Open,
                FileAccess.Read,
                FileShare.Read,
                bufferSize: 16 * 1024,
                options: FileOptions.Asynchronous | FileOptions.SequentialScan);
            using var bytes = new MemoryStream(capacity: 16 * 1024);
            var buffer = new byte[16 * 1024];
            var totalBytes = 0L;
            while (true)
            {
                var read = await stream.ReadAsync(buffer, cancellationToken).ConfigureAwait(false);
                if (read == 0)
                {
                    break;
                }

                totalBytes += read;
                if (totalBytes > MaxManifestBytes)
                {
                    return RuntimeMediaVerificationResult.Failed(Error(
                        RuntimeResourceFailureCode.ManifestTooLarge,
                        "媒体运行资源清单超过大小限制。",
                        retryable: false));
                }

                bytes.Write(buffer, 0, read);
            }

            manifestJson = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true)
                .GetString(bytes.GetBuffer(), 0, checked((int)totalBytes));
            if (manifestJson.Length > 0 && manifestJson[0] == '\uFEFF')
            {
                manifestJson = manifestJson[1..];
            }
        }
        catch (OperationCanceledException)
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.OperationCancelled,
                "媒体运行资源校验已取消。",
                retryable: true));
        }
        catch (DecoderFallbackException)
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.InvalidManifestJson,
                "媒体运行资源清单编码无效。",
                retryable: false));
        }
        catch (IOException)
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.ResourceUnavailable,
                "媒体运行资源暂时不可访问。",
                retryable: true));
        }
        catch (UnauthorizedAccessException)
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.ResourceUnavailable,
                "媒体运行资源暂时不可访问。",
                retryable: true));
        }

        var parsed = Parse(manifestJson);
        if (!parsed.IsSuccess || parsed.Manifest is null)
        {
            return RuntimeMediaVerificationResult.Failed(parsed.Error ?? Error(
                RuntimeResourceFailureCode.InvalidManifestJson,
                "媒体运行资源清单格式无效。",
                retryable: false));
        }

        if (!string.Equals(parsed.Manifest.RuntimeVersion, normalizedVersion, StringComparison.Ordinal))
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.InvalidRuntimeVersion,
                "媒体运行资源版本与目录不一致。",
                retryable: false));
        }

        return await VerifyAsync(installDirectory, parsed.Manifest, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>验证内存清单与安装目录，成功时一次性返回完整资源快照。</summary>
    public static async Task<RuntimeMediaVerificationResult> VerifyAsync(
        string? installationDirectory,
        RuntimeMediaManifestDto? manifest,
        CancellationToken cancellationToken = default)
    {
        var manifestError = ValidateManifest(manifest);
        if (manifestError is not null)
        {
            return RuntimeMediaVerificationResult.Failed(manifestError);
        }

        var directoryError = ValidateInstallationDirectory(installationDirectory, requireExisting: true, out var installDirectory);
        if (directoryError is not null)
        {
            return RuntimeMediaVerificationResult.Failed(directoryError);
        }

        if (manifest is null)
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.InvalidManifestJson,
                "媒体运行资源清单格式无效。",
                retryable: false));
        }

        string versionDirectory;
        string binDirectory;
        try
        {
            versionDirectory = Path.Combine(installDirectory, "runtime", "media", manifest.RuntimeVersion);
            binDirectory = Path.Combine(versionDirectory, "bin");
        }
        catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.InvalidInstallationDirectory,
                "安装目录无效。",
                retryable: false));
        }

        if (!IsWithinDirectory(versionDirectory, installDirectory)
            || !IsWithinDirectory(binDirectory, installDirectory))
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.InvalidInstallationDirectory,
                "媒体运行资源目录不在安装目录内。",
                retryable: false));
        }

        var directoryStateError = ValidateResourceDirectories(installDirectory, versionDirectory, binDirectory);
        if (directoryStateError is not null)
        {
            return RuntimeMediaVerificationResult.Failed(directoryStateError);
        }

        var verified = ImmutableArray.CreateBuilder<VerifiedRuntimeResource>(manifest.Resources.Length);
        try
        {
            foreach (var resource in manifest.Resources)
            {
                cancellationToken.ThrowIfCancellationRequested();
                string resourcePath;
                try
                {
                    resourcePath = Path.GetFullPath(Path.Combine(binDirectory, resource.Name));
                }
                catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
                {
                    return RuntimeMediaVerificationResult.Failed(Error(
                        RuntimeResourceFailureCode.InvalidResourcePath,
                        "媒体运行资源路径无效。",
                        retryable: false));
                }

                if (!IsWithinDirectory(resourcePath, binDirectory))
                {
                    return RuntimeMediaVerificationResult.Failed(Error(
                        RuntimeResourceFailureCode.InvalidResourcePath,
                        "媒体运行资源路径无效。",
                        retryable: false));
                }

                var resourceError = await VerifyResourceAsync(resourcePath, resource, cancellationToken).ConfigureAwait(false);
                if (resourceError is not null)
                {
                    return RuntimeMediaVerificationResult.Failed(resourceError);
                }

                verified.Add(new VerifiedRuntimeResource(
                    resource.Name,
                    resourcePath,
                    resource.SizeBytes,
                    resource.Sha256.ToLowerInvariant()));
            }
        }
        catch (OperationCanceledException)
        {
            return RuntimeMediaVerificationResult.Failed(Error(
                RuntimeResourceFailureCode.OperationCancelled,
                "媒体运行资源校验已取消。",
                retryable: true));
        }

        return RuntimeMediaVerificationResult.Succeeded(new VerifiedMediaRuntime(
            installDirectory,
            versionDirectory,
            binDirectory,
            manifest.RuntimeVersion,
            verified.ToImmutable()));
    }

    /// <summary>仅用于已验证快照的固定资源查询。</summary>
    internal static bool IsAllowedResourceName(string? name) =>
        !string.IsNullOrWhiteSpace(name) && AllowedResourceNames.Contains(name);

    internal static RuntimeResourceError Error(
        RuntimeResourceFailureCode code,
        string message,
        bool retryable) => new(code, message, retryable);

    private static RuntimeResourceError? ValidateJsonShape(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object)
        {
            return Error(RuntimeResourceFailureCode.InvalidManifestJson, "媒体运行资源清单格式无效。", false);
        }

        var rootFields = new HashSet<string>(StringComparer.Ordinal);
        foreach (var property in root.EnumerateObject())
        {
            if (!rootFields.Add(property.Name))
            {
                return Error(RuntimeResourceFailureCode.DuplicateField, "媒体运行资源清单包含重复字段。", false);
            }

            if (property.Name is not ("schema_version" or "runtime_version" or "platform" or "architecture" or "resources"))
            {
                return Error(RuntimeResourceFailureCode.UnknownField, "媒体运行资源清单包含未知字段。", false);
            }
        }

        if (rootFields.Count != 5)
        {
            return Error(RuntimeResourceFailureCode.MissingField, "媒体运行资源清单缺少必要字段。", false);
        }

        if (!root.TryGetProperty("resources", out var resources)
            || resources.ValueKind != JsonValueKind.Array)
        {
            return Error(RuntimeResourceFailureCode.InvalidManifestJson, "媒体运行资源清单格式无效。", false);
        }

        if (resources.GetArrayLength() is 0)
        {
            return Error(RuntimeResourceFailureCode.EmptyResourceList, "媒体运行资源清单不能为空。", false);
        }

        if (resources.GetArrayLength() > MaxResourceCount)
        {
            return Error(RuntimeResourceFailureCode.TooManyResources, "媒体运行资源数量超过限制。", false);
        }

        foreach (var item in resources.EnumerateArray())
        {
            if (item.ValueKind != JsonValueKind.Object)
            {
                return Error(RuntimeResourceFailureCode.InvalidManifestJson, "媒体运行资源清单格式无效。", false);
            }

            var fields = new HashSet<string>(StringComparer.Ordinal);
            foreach (var property in item.EnumerateObject())
            {
                if (!fields.Add(property.Name))
                {
                    return Error(RuntimeResourceFailureCode.DuplicateField, "媒体运行资源清单包含重复字段。", false);
                }

                if (property.Name is not ("name" or "relative_path" or "size_bytes" or "sha256"))
                {
                    return Error(RuntimeResourceFailureCode.UnknownField, "媒体运行资源清单包含未知字段。", false);
                }
            }

            if (fields.Count != 4)
            {
                return Error(RuntimeResourceFailureCode.MissingField, "媒体运行资源项缺少必要字段。", false);
            }
        }

        return null;
    }

    private static RuntimeResourceError? ValidateManifest(RuntimeMediaManifestDto? manifest)
    {
        if (manifest is null)
        {
            return Error(RuntimeResourceFailureCode.InvalidManifestJson, "媒体运行资源清单格式无效。", false);
        }

        if (manifest.SchemaVersion != CurrentSchemaVersion)
        {
            return Error(RuntimeResourceFailureCode.UnsupportedSchemaVersion, "媒体运行资源清单版本不受支持。", false);
        }

        var versionError = ValidateRuntimeVersion(manifest.RuntimeVersion, out _);
        if (versionError is not null)
        {
            return versionError;
        }

        if (!string.Equals(manifest.Platform, ExpectedPlatform, StringComparison.OrdinalIgnoreCase))
        {
            return Error(RuntimeResourceFailureCode.InvalidPlatform, "媒体运行资源平台不受支持。", false);
        }

        if (!string.Equals(manifest.Architecture, ExpectedArchitecture, StringComparison.OrdinalIgnoreCase))
        {
            return Error(RuntimeResourceFailureCode.InvalidArchitecture, "媒体运行资源架构不受支持。", false);
        }

        if (manifest.Resources.IsDefaultOrEmpty)
        {
            return Error(RuntimeResourceFailureCode.EmptyResourceList, "媒体运行资源清单不能为空。", false);
        }

        if (manifest.Resources.Length > MaxResourceCount)
        {
            return Error(RuntimeResourceFailureCode.TooManyResources, "媒体运行资源数量超过限制。", false);
        }

        var names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        long totalBytes = 0;
        foreach (var resource in manifest.Resources)
        {
            if (resource is null)
            {
                return Error(RuntimeResourceFailureCode.InvalidManifestJson, "媒体运行资源项格式无效。", false);
            }

            if (!IsAllowedResourceName(resource.Name))
            {
                return Error(RuntimeResourceFailureCode.UnknownResourceName, "媒体运行资源名称不在允许范围内。", false);
            }

            if (!names.Add(resource.Name))
            {
                return Error(RuntimeResourceFailureCode.DuplicateResource, "媒体运行资源清单包含重复资源。", false);
            }

            var expectedRelativePath = BinPrefix + resource.Name;
            if (!string.Equals(resource.RelativePath, expectedRelativePath, StringComparison.OrdinalIgnoreCase))
            {
                return Error(RuntimeResourceFailureCode.InvalidResourcePath, "媒体运行资源相对路径无效。", false);
            }

            if (resource.SizeBytes <= 0 || resource.SizeBytes > MaxResourceSizeBytes)
            {
                return Error(RuntimeResourceFailureCode.InvalidResourceSize, "媒体运行资源大小声明无效。", false);
            }

            if (!IsSha256(resource.Sha256))
            {
                return Error(RuntimeResourceFailureCode.InvalidSha256, "媒体运行资源哈希声明无效。", false);
            }

            try
            {
                totalBytes = checked(totalBytes + resource.SizeBytes);
            }
            catch (OverflowException)
            {
                return Error(RuntimeResourceFailureCode.ResourceTotalTooLarge, "媒体运行资源总大小超过限制。", false);
            }

            if (totalBytes > MaxTotalResourceBytes)
            {
                return Error(RuntimeResourceFailureCode.ResourceTotalTooLarge, "媒体运行资源总大小超过限制。", false);
            }
        }

        return null;
    }

    private static RuntimeResourceError? ValidateRuntimeVersion(
        string? version,
        out string normalizedVersion)
    {
        normalizedVersion = string.Empty;
        if (string.IsNullOrWhiteSpace(version)
            || version.Length > MaxRuntimeVersionCharacters
            || version.Any(char.IsControl)
            || version[0] is not (>= '0' and <= '9')
                and not (>= 'A' and <= 'Z')
                and not (>= 'a' and <= 'z'))
        {
            return Error(RuntimeResourceFailureCode.InvalidRuntimeVersion, "媒体运行资源版本标识无效。", false);
        }

        var previous = '\0';
        foreach (var character in version)
        {
            var allowed = character is (>= '0' and <= '9')
                or (>= 'A' and <= 'Z')
                or (>= 'a' and <= 'z')
                or '.'
                or '-'
                or '_';
            if (!allowed || (character == '.' && previous == '.'))
            {
                return Error(RuntimeResourceFailureCode.InvalidRuntimeVersion, "媒体运行资源版本标识无效。", false);
            }

            previous = character;
        }

        normalizedVersion = version;
        return null;
    }

    private static RuntimeResourceError? ValidateInstallationDirectory(
        string? installationDirectory,
        bool requireExisting,
        out string normalizedDirectory)
    {
        normalizedDirectory = string.Empty;
        if (string.IsNullOrWhiteSpace(installationDirectory)
            || installationDirectory.Length > MaxInstallationPathCharacters
            || installationDirectory.Any(char.IsControl)
            || !Path.IsPathFullyQualified(installationDirectory))
        {
            return Error(RuntimeResourceFailureCode.InvalidInstallationDirectory, "安装目录必须是有效的绝对目录。", false);
        }

        try
        {
            normalizedDirectory = Path.TrimEndingDirectorySeparator(Path.GetFullPath(installationDirectory));
        }
        catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return Error(RuntimeResourceFailureCode.InvalidInstallationDirectory, "安装目录必须是有效的绝对目录。", false);
        }

        if (normalizedDirectory.Length > MaxInstallationPathCharacters)
        {
            return Error(RuntimeResourceFailureCode.InvalidInstallationDirectory, "安装目录必须是有效的绝对目录。", false);
        }

        if (requireExisting && !Directory.Exists(normalizedDirectory))
        {
            return Error(RuntimeResourceFailureCode.InstallationDirectoryMissing, "安装目录不存在。", true);
        }

        if (Directory.Exists(normalizedDirectory) && HasReparsePoint(normalizedDirectory))
        {
            return Error(RuntimeResourceFailureCode.ReparsePointNotAllowed, "安装目录不能通过重解析点访问。", false);
        }

        return null;
    }

    private static RuntimeResourceError? ValidateResourceDirectories(
        string installationDirectory,
        string versionDirectory,
        string binDirectory)
    {
        var runtimeDirectory = Path.Combine(installationDirectory, "runtime");
        var mediaDirectory = Path.Combine(runtimeDirectory, "media");
        foreach (var directory in new[] { runtimeDirectory, mediaDirectory, versionDirectory, binDirectory })
        {
            if (!Directory.Exists(directory))
            {
                return Error(RuntimeResourceFailureCode.ResourceDirectoryMissing, "媒体运行资源目录不存在。", true);
            }

            if (HasReparsePoint(directory))
            {
                return Error(RuntimeResourceFailureCode.ReparsePointNotAllowed, "媒体运行资源不能通过重解析点加载。", false);
            }
        }

        return null;
    }

    private static async Task<RuntimeResourceError?> VerifyResourceAsync(
        string path,
        RuntimeMediaResourceDto resource,
        CancellationToken cancellationToken)
    {
        if (!File.Exists(path))
        {
            return Error(RuntimeResourceFailureCode.ResourceMissing, "媒体运行资源文件不存在。", true);
        }

        FileStream stream;
        try
        {
            var attributes = File.GetAttributes(path);
            if (attributes.HasFlag(FileAttributes.Directory))
            {
                return Error(RuntimeResourceFailureCode.ResourceIsDirectory, "媒体运行资源不是文件。", false);
            }

            if (attributes.HasFlag(FileAttributes.ReparsePoint))
            {
                return Error(RuntimeResourceFailureCode.ReparsePointNotAllowed, "媒体运行资源不能通过重解析点加载。", false);
            }

            stream = new FileStream(
                path,
                FileMode.Open,
                FileAccess.Read,
                FileShare.Read,
                bufferSize: 64 * 1024,
                options: FileOptions.Asynchronous | FileOptions.SequentialScan);
        }
        catch (FileNotFoundException)
        {
            return Error(RuntimeResourceFailureCode.ResourceMissing, "媒体运行资源文件不存在。", true);
        }
        catch (DirectoryNotFoundException)
        {
            return Error(RuntimeResourceFailureCode.ResourceMissing, "媒体运行资源文件不存在。", true);
        }
        catch (UnauthorizedAccessException)
        {
            return Error(RuntimeResourceFailureCode.ResourceUnavailable, "媒体运行资源暂时不可访问。", true);
        }
        catch (IOException)
        {
            return Error(RuntimeResourceFailureCode.ResourceUnavailable, "媒体运行资源暂时不可访问。", true);
        }

        try
        {
            await using (stream.ConfigureAwait(false))
            {
                if (stream.Length != resource.SizeBytes)
                {
                    return Error(RuntimeResourceFailureCode.ResourceSizeMismatch, "媒体运行资源大小校验失败。", false);
                }

                byte[] actualHash;
                try
                {
                    actualHash = await SHA256.HashDataAsync(stream, cancellationToken).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    return Error(RuntimeResourceFailureCode.OperationCancelled, "媒体运行资源校验已取消。", true);
                }
                catch (IOException)
                {
                    return Error(RuntimeResourceFailureCode.ResourceUnavailable, "媒体运行资源暂时不可访问。", true);
                }

                if (stream.Length != resource.SizeBytes)
                {
                    return Error(RuntimeResourceFailureCode.ResourceSizeMismatch, "媒体运行资源大小校验失败。", false);
                }

                var expectedHash = Convert.FromHexString(resource.Sha256);
                if (!CryptographicOperations.FixedTimeEquals(actualHash, expectedHash))
                {
                    return Error(RuntimeResourceFailureCode.ResourceHashMismatch, "媒体运行资源完整性校验失败。", false);
                }
            }
        }
        catch (UnauthorizedAccessException)
        {
            return Error(RuntimeResourceFailureCode.ResourceUnavailable, "媒体运行资源暂时不可访问。", true);
        }
        catch (IOException)
        {
            return Error(RuntimeResourceFailureCode.ResourceUnavailable, "媒体运行资源暂时不可访问。", true);
        }

        return null;
    }

    private static bool IsSha256(string? value)
    {
        if (value is null || value.Length != 64)
        {
            return false;
        }

        foreach (var character in value)
        {
            if (!Uri.IsHexDigit(character))
            {
                return false;
            }
        }

        return true;
    }

    private static bool IsWithinDirectory(string candidate, string directory)
    {
        try
        {
            var fullCandidate = Path.GetFullPath(candidate);
            var fullDirectory = Path.TrimEndingDirectorySeparator(Path.GetFullPath(directory));
            if (string.Equals(fullCandidate, fullDirectory, StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }

            var prefix = fullDirectory.EndsWith(Path.DirectorySeparatorChar)
                || fullDirectory.EndsWith(Path.AltDirectorySeparatorChar)
                ? fullDirectory
                : fullDirectory + Path.DirectorySeparatorChar;
            return fullCandidate.StartsWith(prefix, StringComparison.OrdinalIgnoreCase);
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (NotSupportedException)
        {
            return false;
        }
        catch (PathTooLongException)
        {
            return false;
        }
    }

    private static bool HasReparsePoint(string path)
    {
        try
        {
            return File.GetAttributes(path).HasFlag(FileAttributes.ReparsePoint);
        }
        catch (FileNotFoundException)
        {
            return false;
        }
        catch (DirectoryNotFoundException)
        {
            return false;
        }
        catch (UnauthorizedAccessException)
        {
            return true;
        }
        catch (IOException)
        {
            return true;
        }
    }
}
