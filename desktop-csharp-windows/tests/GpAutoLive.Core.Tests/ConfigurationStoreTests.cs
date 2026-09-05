using System.Text;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class ConfigurationStoreTests
{
    private readonly List<string> _temporaryDirectories = [];

    [TestCleanup]
    public void Cleanup()
    {
        foreach (var directory in _temporaryDirectories)
        {
            try
            {
                if (Directory.Exists(directory))
                {
                    Directory.Delete(directory, recursive: true);
                }
            }
            catch (IOException)
            {
                // 测试清理失败不覆盖主断言。
            }
        }
    }

    [TestMethod]
    public async Task Ini_round_trip_uses_whitelist_and_leaves_no_temp_file()
    {
        var path = CreatePath("config", "app.ini");
        var store = new IniUserPreferencesStore(path);
        var expected = new UserPreferences
        {
            WindowWidth = 1440,
            WindowHeight = 900,
            WindowLeft = -10.5,
            WindowTop = 20,
            Theme = "LIGHT",
            Language = "zh-cn",
            EffectsPanelExpanded = false,
            PerformanceSamplingEnabled = true,
            LastOutputMode = "RTMP"
        };

        await store.SaveAsync(expected);
        var actual = await store.ReadAsync();

        Assert.AreEqual(1440, actual.WindowWidth);
        Assert.AreEqual(900, actual.WindowHeight);
        Assert.AreEqual(-10.5, actual.WindowLeft);
        Assert.AreEqual(20, actual.WindowTop);
        Assert.AreEqual("light", actual.Theme);
        Assert.AreEqual("zh-CN", actual.Language);
        Assert.IsFalse(actual.EffectsPanelExpanded);
        Assert.IsTrue(actual.PerformanceSamplingEnabled);
        Assert.AreEqual("rtmp", actual.LastOutputMode);
        Assert.AreEqual(0, Directory.GetFiles(Path.GetDirectoryName(path)!, "*.tmp", SearchOption.TopDirectoryOnly).Length);
    }

    [TestMethod]
    public async Task Ini_rejects_unknown_secret_key_and_preserves_previous_file()
    {
        var path = CreatePath("config", "app.ini");
        var store = new IniUserPreferencesStore(path);
        await store.SaveAsync(UserPreferences.Defaults);
        var original = await File.ReadAllTextAsync(path);

        await File.AppendAllTextAsync(path, Environment.NewLine + "[secrets]" + Environment.NewLine + "api_key=do-not-store");

        await ThrowsAsync<ConfigurationValidationException>(() => store.ReadAsync());
        var afterFailedRead = await File.ReadAllTextAsync(path);
        Assert.IsTrue(afterFailedRead.Contains("do-not-store", StringComparison.Ordinal));

        await store.SaveAsync(UserPreferences.Defaults);
        var rewritten = await File.ReadAllTextAsync(path);
        Assert.AreEqual(original, rewritten);
        Assert.IsFalse(rewritten.Contains("api_key", StringComparison.OrdinalIgnoreCase));
        Assert.IsFalse(rewritten.Contains("do-not-store", StringComparison.Ordinal));
    }

    [TestMethod]
    public async Task Ini_rejects_out_of_range_and_missing_values_without_replacing_valid_file()
    {
        var path = CreatePath("config", "app.ini");
        var store = new IniUserPreferencesStore(path);
        await store.SaveAsync(UserPreferences.Defaults);
        var original = await File.ReadAllTextAsync(path);

        var invalid = original.Replace("width=1280", "width=959", StringComparison.Ordinal);
        await File.WriteAllTextAsync(path, invalid, Encoding.UTF8);

        await ThrowsAsync<ConfigurationValidationException>(() => store.ReadAsync());
        await store.SaveAsync(UserPreferences.Defaults);
        Assert.AreEqual(original, await File.ReadAllTextAsync(path));
    }

    [TestMethod]
    public async Task Ini_empty_optional_coordinates_are_read_as_null()
    {
        var path = CreatePath("config", "app.ini");
        var store = new IniUserPreferencesStore(path);

        await store.SaveAsync(UserPreferences.Defaults);
        var actual = await store.ReadAsync();

        Assert.IsNull(actual.WindowLeft);
        Assert.IsNull(actual.WindowTop);
    }

    [TestMethod]
    public async Task Versioned_json_round_trip_is_atomic_and_rejects_future_or_unknown_data()
    {
        var path = CreatePath("profiles", "media.json");
        var store = new VersionedJsonStore<ProfileData>(path, currentSchemaVersion: 1);
        await store.WriteAsync(new ProfileData("demo", 2));

        var document = await store.ReadAsync();
        Assert.IsNotNull(document);
        Assert.AreEqual(1, document.SchemaVersion);
        Assert.AreEqual("demo", document.Data!.Name);
        Assert.AreEqual(2, document.Data.Revision);
        Assert.AreEqual(0, Directory.GetFiles(Path.GetDirectoryName(path)!, "*.tmp", SearchOption.TopDirectoryOnly).Length);

        await File.WriteAllTextAsync(path, "{\"schema_version\":2,\"data\":{\"name\":\"future\",\"revision\":3}}", Encoding.UTF8);
        await ThrowsAsync<ConfigurationValidationException>(() => store.ReadAsync());

        await File.WriteAllTextAsync(path, "{\"schema_version\":1,\"data\":{\"name\":\"known\",\"revision\":3,\"api_key\":\"do-not-store\"}}", Encoding.UTF8);
        await ThrowsAsync<ConfigurationValidationException>(() => store.ReadAsync());

        await File.WriteAllTextAsync(path, "{\"schema_version\":1,\"data\":{\"name\":\"rtmp\",\"revision\":3,\"target_url\":\"rtmp://example.invalid/live\"}}", Encoding.UTF8);
        await ThrowsAsync<ConfigurationValidationException>(() => store.ReadAsync());
    }

    [TestMethod]
    public async Task Versioned_json_rejects_null_payload_and_does_not_create_file()
    {
        var path = CreatePath("profiles", "empty.json");
        var store = new VersionedJsonStore<ProfileData>(path, currentSchemaVersion: 1);

        await ThrowsAsync<ArgumentNullException>(() => store.WriteAsync(null!));
        Assert.IsFalse(File.Exists(path));
        Assert.IsNull(await store.ReadAsync());
    }

    [TestMethod]
    public async Task Versioned_json_never_writes_sensitive_property_names()
    {
        var path = CreatePath("profiles", "unsafe.json");
        var store = new VersionedJsonStore<UnsafeProfile>(path, currentSchemaVersion: 1);

        await ThrowsAsync<ConfigurationValidationException>(() => store.WriteAsync(new UnsafeProfile("demo", "test-secret")));

        Assert.IsFalse(File.Exists(path));
    }

    private string CreatePath(params string[] parts)
    {
        var directory = Path.Combine(Path.GetTempPath(), "GpAutoLive.CSharp.Tests", Guid.NewGuid().ToString("N"));
        _temporaryDirectories.Add(directory);
        return Path.Combine([directory, .. parts]);
    }

    public sealed record ProfileData(string Name, int Revision);

    public sealed record UnsafeProfile(string Name, string ApiKey);

    private static async Task ThrowsAsync<TException>(Func<Task> action)
        where TException : Exception
    {
        try
        {
            await action();
        }
        catch (TException)
        {
            return;
        }

        Assert.Fail($"应抛出 {typeof(TException).Name}。");
    }
}
