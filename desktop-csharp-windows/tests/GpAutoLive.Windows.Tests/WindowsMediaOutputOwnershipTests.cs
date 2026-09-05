namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMediaOutputOwnershipTests
{
    [TestMethod]
    public void First_lease_wins_second_is_rejected_and_release_allows_retry()
    {
        var mutexName = $"Local\\GpAutoLive.Tests.{Guid.NewGuid():N}";
        var first = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName,
            referenceClientProbe: static () => false);

        Assert.IsTrue(first.IsSuccess, first.Code.ToString());
        Assert.IsNotNull(first.Lease);
        using var firstLease = first.Lease!;

        var second = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName,
            referenceClientProbe: static () => false);
        Assert.AreEqual(WindowsMediaOutputOwnershipCode.AlreadyOwned, second.Code);
        Assert.IsNull(second.Lease);

        firstLease.Dispose();
        var retry = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName,
            referenceClientProbe: static () => false);
        Assert.IsTrue(retry.IsSuccess, retry.Code.ToString());
        retry.Lease!.Dispose();
    }

    [TestMethod]
    public void Reference_client_probe_blocks_before_mutex_creation()
    {
        var mutexName = $"Local\\GpAutoLive.Tests.{Guid.NewGuid():N}";
        var result = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName,
            referenceClientProbe: static () => true);

        Assert.AreEqual(WindowsMediaOutputOwnershipCode.ReferenceClientRunning, result.Code);
        Assert.IsNull(result.Lease);
    }

    [TestMethod]
    public void Invalid_mutex_name_fails_closed()
    {
        var result = WindowsMediaOutputOwnershipLease.TryAcquire(
            "GpAutoLive.Tests.Invalid",
            referenceClientProbe: static () => false);

        Assert.AreEqual(WindowsMediaOutputOwnershipCode.InvalidMutexName, result.Code);
        Assert.IsNull(result.Lease);
    }

    [TestMethod]
    public void Probe_failure_fails_closed()
    {
        var mutexName = $"Local\\GpAutoLive.Tests.{Guid.NewGuid():N}";
        var result = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName,
            referenceClientProbe: static () => throw new InvalidOperationException());

        Assert.AreEqual(WindowsMediaOutputOwnershipCode.ReferenceClientProbeFailed, result.Code);
        Assert.IsNull(result.Lease);
    }

    [TestMethod]
    public void Reference_client_appearing_after_mutex_acquisition_is_rejected_and_mutex_is_released()
    {
        var mutexName = $"Local\\GpAutoLive.Tests.{Guid.NewGuid():N}";
        var probeCount = 0;
        var result = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName,
            referenceClientProbe: () => Interlocked.Increment(ref probeCount) == 2);

        Assert.AreEqual(WindowsMediaOutputOwnershipCode.ReferenceClientRunning, result.Code);
        Assert.IsNull(result.Lease);
        Assert.AreEqual(2, probeCount);

        var retry = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName,
            referenceClientProbe: static () => false);
        Assert.IsTrue(retry.IsSuccess, retry.Code.ToString());
        retry.Lease!.Dispose();
    }

    [TestMethod]
    public void Probe_failure_after_mutex_acquisition_releases_mutex()
    {
        var mutexName = $"Local\\GpAutoLive.Tests.{Guid.NewGuid():N}";
        var probeCount = 0;
        var result = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName,
            referenceClientProbe: () =>
            {
                if (Interlocked.Increment(ref probeCount) == 2)
                {
                    throw new InvalidOperationException();
                }

                return false;
            });

        Assert.AreEqual(WindowsMediaOutputOwnershipCode.ReferenceClientProbeFailed, result.Code);
        Assert.IsNull(result.Lease);

        var retry = WindowsMediaOutputOwnershipLease.TryAcquire(
            mutexName,
            referenceClientProbe: static () => false);
        Assert.IsTrue(retry.IsSuccess, retry.Code.ToString());
        retry.Lease!.Dispose();
    }
}
