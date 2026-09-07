namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMediaOutputOwnershipTests
{
    [TestMethod]
    public void First_lease_wins_second_is_rejected_and_release_allows_retry()
    {
        var mutexName = $"Local\\GpAutoLive.Tests.{Guid.NewGuid():N}";
        var first = WindowsMediaOutputOwnershipLease.TryAcquire(mutexName);

        Assert.IsTrue(first.IsSuccess, first.Code.ToString());
        Assert.IsNotNull(first.Lease);
        using var firstLease = first.Lease!;

        var second = WindowsMediaOutputOwnershipLease.TryAcquire(mutexName);
        Assert.AreEqual(WindowsMediaOutputOwnershipCode.AlreadyOwned, second.Code);
        Assert.IsNull(second.Lease);

        firstLease.Dispose();
        var retry = WindowsMediaOutputOwnershipLease.TryAcquire(mutexName);
        Assert.IsTrue(retry.IsSuccess, retry.Code.ToString());
        retry.Lease!.Dispose();
    }

    [TestMethod]
    public void Invalid_mutex_name_fails_closed()
    {
        var result = WindowsMediaOutputOwnershipLease.TryAcquire("GpAutoLive.Tests.Invalid");

        Assert.AreEqual(WindowsMediaOutputOwnershipCode.InvalidMutexName, result.Code);
        Assert.IsNull(result.Lease);
    }

    [TestMethod]
    public void Kernel_mutex_contention_is_rejected_and_release_allows_retry()
    {
        if (!OperatingSystem.IsWindows())
        {
            Assert.Inconclusive("命名 Mutex 门禁仅在 Windows 上验证。");
        }

        var mutexName = $"Local\\GpAutoLive.Tests.{Guid.NewGuid():N}";
        using var ready = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        Exception? ownerFailure = null;
        var ownerThread = new Thread(() =>
        {
            try
            {
                using var mutex = new Mutex(initiallyOwned: false, mutexName);
                Assert.IsTrue(mutex.WaitOne(TimeSpan.FromSeconds(5)));
                ready.Set();
                release.Wait(TimeSpan.FromSeconds(5));
                mutex.ReleaseMutex();
            }
            catch (Exception exception)
            {
                ownerFailure = exception;
                ready.Set();
            }
        });

        ownerThread.Start();
        try
        {
            Assert.IsTrue(ready.Wait(TimeSpan.FromSeconds(5)), "内核 Mutex 持有线程未就绪。");
            Assert.IsNull(ownerFailure, ownerFailure?.ToString());

            var blocked = WindowsMediaOutputOwnershipLease.TryAcquire(
                mutexName);
            Assert.AreEqual(WindowsMediaOutputOwnershipCode.AlreadyOwned, blocked.Code);
            Assert.IsNull(blocked.Lease);
        }
        finally
        {
            release.Set();
            Assert.IsTrue(ownerThread.Join(TimeSpan.FromSeconds(5)), "内核 Mutex 持有线程未退出。");
        }

        Assert.IsNull(ownerFailure, ownerFailure?.ToString());
        var retry = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName);
        Assert.IsTrue(retry.IsSuccess, retry.Code.ToString());
        retry.Lease!.Dispose();
    }

    [TestMethod]
    public void Abandoned_kernel_mutex_is_taken_over_and_released_cleanly()
    {
        if (!OperatingSystem.IsWindows())
        {
            Assert.Inconclusive("命名 Mutex 门禁仅在 Windows 上验证。");
        }

        var mutexName = $"Local\\GpAutoLive.Tests.{Guid.NewGuid():N}";
        using var ready = new ManualResetEventSlim();
        var ownerThread = new Thread(() =>
        {
            using var mutex = new Mutex(initiallyOwned: false, mutexName);
            Assert.IsTrue(mutex.WaitOne(TimeSpan.FromSeconds(5)));
            ready.Set();
        });

        ownerThread.Start();
        Assert.IsTrue(ready.Wait(TimeSpan.FromSeconds(5)), "异常退出模拟线程未取得 Mutex。");
        Assert.IsTrue(ownerThread.Join(TimeSpan.FromSeconds(5)), "异常退出模拟线程未退出。");

        var result = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName);
        Assert.IsTrue(result.IsSuccess, result.Code.ToString());
        result.Lease!.Dispose();
    }
}
