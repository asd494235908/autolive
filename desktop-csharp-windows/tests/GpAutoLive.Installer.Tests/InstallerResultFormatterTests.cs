namespace GpAutoLive.Installer.Tests;

[TestClass]
public sealed class InstallerResultFormatterTests
{
    [TestMethod]
    public void VerifiedPackageReport_IsAcceptedWithoutStatusField()
    {
        const string json = """
            {"schema_version":1,"package_root_files":[{}],"gpu_manifest_files":[{}],"winrt_manifest_files":[{}],"media_manifest_files":[{}],"require_signed":true}
            """;

        var summary = InstallerResultFormatter.Format(
            InstallerOperation.VerifyPackage,
            new(0, json, string.Empty, false, false));

        Assert.IsTrue(summary.IsHealthy);
        StringAssert.Contains(summary.Text, "发布包校验通过");
    }

    [TestMethod]
    public void HealthyState_ProjectsOnlyUsefulFields()
    {
        const string json = """
            {"schema_version":1,"status":"healthy","code":"installation_verified","active":{"version":"v84","status":"verified"},"rollback":{"available":true,"version":"v83","status":"verified"},"versions":[{"version":"v83","active":false,"rollback_candidate":true,"status":"verified"},{"version":"v84","active":true,"rollback_candidate":false,"status":"verified"}]}
            """;

        var summary = InstallerResultFormatter.Format(
            InstallerOperation.CheckInstallState,
            new(0, json, string.Empty, false, false));

        Assert.IsTrue(summary.IsHealthy);
        StringAssert.Contains(summary.Text, "活动版本：v84");
        StringAssert.Contains(summary.Text, "可回滚版本：v83");
        Assert.IsFalse(summary.Text.Contains("versions", StringComparison.Ordinal));
    }

    [TestMethod]
    public void RuntimeMissing_IsNotReportedAsReady()
    {
        var summary = InstallerResultFormatter.Format(
            InstallerOperation.CheckRuntime,
            new(0, "{\"status\":\"runtime_missing\"}", string.Empty, false, false));

        Assert.IsFalse(summary.IsHealthy);
        StringAssert.Contains(summary.Text, ".NET 10 Windows Desktop Runtime");
    }

    [TestMethod]
    public void Failure_RedactsLocalAndNetworkPaths()
    {
        var summary = InstallerResultFormatter.Format(
            InstallerOperation.InstallOrUpgrade,
            new(1, string.Empty, @"failed C:\Users\someone\secret and \\server\share\item", false, false));

        Assert.IsFalse(summary.IsHealthy);
        StringAssert.Contains(summary.Text, "<本地路径>");
        StringAssert.Contains(summary.Text, "<网络路径>");
        Assert.IsFalse(summary.Text.Contains("someone", StringComparison.Ordinal));
    }

    [TestMethod]
    public void Cancelled_RequiresStateRefresh()
    {
        var summary = InstallerResultFormatter.Format(
            InstallerOperation.Rollback,
            new(-1, string.Empty, "cancelled", true, false));

        Assert.IsFalse(summary.IsHealthy);
        StringAssert.Contains(summary.Text, "刷新安装状态");
    }

    [TestMethod]
    public void UnknownStatus_IsNotReportedHealthy()
    {
        var summary = InstallerResultFormatter.Format(
            InstallerOperation.InstallOrUpgrade,
            new(0, "{\"status\":\"unexpected\"}", string.Empty, false, false));

        Assert.IsFalse(summary.IsHealthy);
    }

    [TestMethod]
    public void HealthyStateWithoutConsistentVersionSnapshot_IsRejected()
    {
        const string json = "{\"schema_version\":1,\"status\":\"healthy\",\"active\":{\"version\":\"v84\"}}";

        var summary = InstallerResultFormatter.Format(
            InstallerOperation.CheckInstallState,
            new(0, json, string.Empty, false, false));

        Assert.IsFalse(summary.IsHealthy);
    }

    [TestMethod]
    public void NonObjectJson_IsRejectedWithoutThrowing()
    {
        var summary = InstallerResultFormatter.Format(
            InstallerOperation.VerifyPackage,
            new(0, "[]", string.Empty, false, false));

        Assert.IsFalse(summary.IsHealthy);
        StringAssert.Contains(summary.Text, "无法识别");
    }
}
