using GpAutoLive.App.Features.Runtime;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class ExternalWinRtRuntimeResolverTests
{
    [TestMethod]
    public void Candidate_path_is_bounded_to_external_winrt_runtime_directory()
    {
        var path = ExternalWinRtRuntimeResolver.GetCandidatePath("C:\\GpAutoLive", "WinRT.Runtime");

        StringAssert.EndsWith(path, System.IO.Path.Combine("runtime", "winrt", "WinRT.Runtime.dll"));
        Assert.IsFalse(path.Contains("..", StringComparison.Ordinal), "The resolver must not construct a traversal path.");
    }

    [TestMethod]
    public void Unsupported_assembly_name_is_rejected()
    {
        Assert.ThrowsExactly<ArgumentException>(() =>
            ExternalWinRtRuntimeResolver.GetCandidatePath("C:\\GpAutoLive", "..\\outside"));
    }

    [TestMethod]
    public void Vortice_dependencies_are_bounded_to_external_gpu_runtime_directory()
    {
        var path = ExternalWinRtRuntimeResolver.GetCandidatePath("C:\\GpAutoLive", "Vortice.Direct3D11");

        StringAssert.EndsWith(path, System.IO.Path.Combine("runtime", "gpu", "Vortice.Direct3D11.dll"));
        Assert.IsFalse(path.Contains("..", StringComparison.Ordinal));

        var dxgiPath = ExternalWinRtRuntimeResolver.GetCandidatePath("C:\\GpAutoLive", "Vortice.DXGI");
        StringAssert.EndsWith(dxgiPath, System.IO.Path.Combine("runtime", "gpu", "Vortice.DXGI.dll"));
        Assert.IsFalse(dxgiPath.Contains("..", StringComparison.Ordinal));
    }
}
