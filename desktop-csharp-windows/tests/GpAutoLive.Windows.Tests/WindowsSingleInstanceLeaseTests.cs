namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsSingleInstanceLeaseTests
{
    [TestMethod]
    public void A_second_lease_for_the_same_lock_path_is_rejected_until_the_first_is_disposed()
    {
        if (!OperatingSystem.IsWindows())
        {
            Assert.Inconclusive("单实例锁文件门禁仅在 Windows 上验证。");
        }

        var directory = Path.Combine(Path.GetTempPath(), $"gpautolive-single-instance-{Guid.NewGuid():N}");
        Directory.CreateDirectory(directory);
        var lockPath = Path.Combine(directory, "instance.lock");
        try
        {
            var first = WindowsSingleInstanceLease.TryAcquire(lockPath);
            Assert.IsTrue(first.IsSuccess);
            Assert.IsNotNull(first.Lease);

            var second = WindowsSingleInstanceLease.TryAcquire(lockPath);
            Assert.IsFalse(second.IsSuccess);
            Assert.AreEqual(WindowsSingleInstanceCode.AlreadyOwned, second.Code);

            first.Lease.Dispose();

            var afterRelease = WindowsSingleInstanceLease.TryAcquire(lockPath);
            Assert.IsTrue(afterRelease.IsSuccess);
            afterRelease.Lease!.Dispose();
        }
        finally
        {
            try
            {
                Directory.Delete(directory, recursive: true);
            }
            catch (IOException)
            {
                // 锁文件仍被系统占用时保留临时目录，避免清理异常覆盖断言。
            }
        }
    }
}
