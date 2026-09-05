using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraSurfaceBindingTests
{
    [TestMethod]
    public void Zero_handle_is_rejected_without_advancing_generation()
    {
        using var binding = new WindowsVirtualCameraSurfaceBinding();
        var initial = binding.Snapshot;

        var result = binding.Bind(0);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSurfaceBindingCode.InvalidHandle, result.Code);
        Assert.AreEqual(initial, result.Snapshot);
    }

    [TestMethod]
    public void Same_handle_is_idempotent_but_changed_handle_invalidates_old_generation()
    {
        using var binding = new WindowsVirtualCameraSurfaceBinding();

        var first = binding.Bind(101);
        var same = binding.Bind(101);
        var changed = binding.Bind(202);

        Assert.IsTrue(first.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSurfaceBindingCode.Bound, first.Code);
        Assert.AreEqual(WindowsVirtualCameraSurfaceBindingCode.Unchanged, same.Code);
        Assert.AreEqual(first.Snapshot.Generation, same.Snapshot.Generation);
        Assert.AreEqual(WindowsVirtualCameraSurfaceBindingCode.Bound, changed.Code);
        Assert.IsTrue(changed.Snapshot.Generation > first.Snapshot.Generation);
        Assert.IsFalse(binding.IsCurrent(101, first.Snapshot.Generation));
        Assert.IsTrue(binding.IsCurrent(202, changed.Snapshot.Generation));
    }

    [TestMethod]
    public void Unbind_invalidates_current_generation_and_dispose_is_idempotent()
    {
        var binding = new WindowsVirtualCameraSurfaceBinding();
        var bound = binding.Bind(303);

        var unbound = binding.Unbind();
        binding.Dispose();
        var afterDispose = binding.Bind(404);

        Assert.IsTrue(unbound.IsSuccess);
        Assert.IsNull(unbound.Snapshot.WindowId);
        Assert.IsFalse(unbound.Snapshot.IsBound);
        Assert.IsTrue(unbound.Snapshot.Generation > bound.Snapshot.Generation);
        Assert.IsFalse(afterDispose.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSurfaceBindingCode.Closed, afterDispose.Code);
        Assert.IsTrue(afterDispose.Snapshot.IsClosed);
    }
}
