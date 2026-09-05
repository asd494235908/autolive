using System.IO;
using GpAutoLive.App.Features.Settings;
using GpAutoLive.Core.Configuration;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class DesktopPreferencesCoordinatorTests
{
    [TestMethod]
    public async Task Missing_preferences_load_defaults_without_warning()
    {
        using var fixture = PreferencesFixture.Create();
        var coordinator = new DesktopPreferencesCoordinator(
            new IniUserPreferencesStore(fixture.Path));

        var warning = await coordinator.LoadAsync();

        Assert.IsNull(warning);
        Assert.IsTrue(coordinator.IsLoaded);
        Assert.AreEqual(UserPreferences.Defaults, coordinator.Current);
    }

    [TestMethod]
    public async Task Invalid_preferences_fail_closed_to_defaults()
    {
        using var fixture = PreferencesFixture.Create();
        Directory.CreateDirectory(Path.GetDirectoryName(fixture.Path)!);
        await File.WriteAllTextAsync(fixture.Path, "[unexpected]\nvalue=true\n");
        var coordinator = new DesktopPreferencesCoordinator(
            new IniUserPreferencesStore(fixture.Path));

        var warning = await coordinator.LoadAsync();

        Assert.AreEqual("本地偏好格式无效，已使用默认设置", warning);
        Assert.IsTrue(coordinator.IsLoaded);
        Assert.AreEqual(UserPreferences.Defaults, coordinator.Current);
    }

    private sealed class PreferencesFixture : IDisposable
    {
        private PreferencesFixture(string root)
        {
            Root = root;
            Path = System.IO.Path.Combine(root, "config", "app.ini");
        }

        public string Root { get; }

        public string Path { get; }

        public static PreferencesFixture Create() =>
            new(System.IO.Path.Combine(
                System.IO.Path.GetTempPath(),
                "gpautolive-preferences-tests",
                Guid.NewGuid().ToString("N")));

        public void Dispose()
        {
            if (Directory.Exists(Root))
            {
                Directory.Delete(Root, recursive: true);
            }
        }
    }
}
