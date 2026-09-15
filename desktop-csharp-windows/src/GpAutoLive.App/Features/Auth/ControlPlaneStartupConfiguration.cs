using System.IO;
using System.Security;

namespace GpAutoLive.App.Features.Auth;

internal sealed record ControlPlaneStartupConfiguration(
    string? BaseUriText,
    bool AllowsDevelopmentHttp,
    bool UsesTestCredentialStore)
{
    internal const string ProductionProfile = "production-v1";
    internal const string CloudTestProfile = "cloud-test-v1";
    internal const string ProfileFileName = "GpAutoLive.control-plane-profile";
    internal const string FixedCloudTestBaseUri = "http://101.96.208.132:9090";

    private const string InvalidProfile = "invalid-v1";
    private const string BaseUriVariable = "AUTOLIVE_CONTROL_PLANE_BASE_URI";
    private const string EnvironmentVariable = "AUTOLIVE_CONTROL_PLANE_ENV";

    internal static ControlPlaneStartupConfiguration Load() =>
        Resolve(
            ReadPackagedProfile(),
            System.Environment.GetEnvironmentVariable(EnvironmentVariable),
            System.Environment.GetEnvironmentVariable(BaseUriVariable));

    internal static ControlPlaneStartupConfiguration Resolve(
        string? packagedProfile,
        string? processEnvironment,
        string? processBaseUri)
    {
        var profile = Normalize(packagedProfile);
        var environment = Normalize(processEnvironment);
        var baseUri = Normalize(processBaseUri);

        if (string.Equals(profile, CloudTestProfile, StringComparison.Ordinal))
        {
            return string.Equals(environment, "offline", StringComparison.OrdinalIgnoreCase)
                ? new(null, false, true)
                : new(FixedCloudTestBaseUri, true, true);
        }

        if (string.Equals(profile, ProductionProfile, StringComparison.Ordinal))
        {
            if (string.Equals(environment, "offline", StringComparison.OrdinalIgnoreCase)
                || IsDevelopmentTestEnvironment(environment))
            {
                return new(null, false, false);
            }

            return new(baseUri, false, false);
        }

        if (profile is not null)
        {
            return new(null, false, false);
        }

        if (string.Equals(environment, "offline", StringComparison.OrdinalIgnoreCase))
        {
            return new(null, false, false);
        }

        var allowsDevelopmentHttp = IsDevelopmentTestEnvironment(environment);
        if (allowsDevelopmentHttp && baseUri is null)
        {
            baseUri = FixedCloudTestBaseUri;
        }

        return new(baseUri, allowsDevelopmentHttp, allowsDevelopmentHttp);
    }

    private static string? ReadPackagedProfile()
    {
        var path = Path.Combine(AppContext.BaseDirectory, ProfileFileName);
        try
        {
            if (!File.Exists(path))
            {
                return null;
            }

            var file = new FileInfo(path);
            if ((file.Attributes & FileAttributes.ReparsePoint) != 0 || file.Length is <= 0 or > 64)
            {
                return InvalidProfile;
            }

            var value = File.ReadAllText(path).Trim();
            return value is ProductionProfile or CloudTestProfile ? value : InvalidProfile;
        }
        catch (Exception exception) when (exception is IOException or UnauthorizedAccessException or SecurityException)
        {
            return InvalidProfile;
        }
    }

    private static bool IsDevelopmentTestEnvironment(string? environment) =>
        string.Equals(environment, "test", StringComparison.OrdinalIgnoreCase)
        || string.Equals(environment, "development", StringComparison.OrdinalIgnoreCase);

    private static string? Normalize(string? value) =>
        string.IsNullOrWhiteSpace(value) ? null : value.Trim();
}
