using System.IO;
using System.Reflection;
using System.Runtime.Loader;

namespace GpAutoLive.App.Features.Runtime;

/// <summary>
/// Loads the Windows SDK projection assemblies from the external runtime tree.
/// Keeping them outside the application root keeps the shell directory and the
/// primary executable small while preserving normal assembly probing in dev.
/// </summary>
public static class ExternalWinRtRuntimeResolver
{
    private static readonly HashSet<string> SupportedAssemblies = new(StringComparer.OrdinalIgnoreCase)
    {
        "Microsoft.Windows.SDK.NET",
        "WinRT.Runtime",
        "Vortice.Direct3D11",
        "Vortice.DirectX",
        "Vortice.DXGI",
        "Vortice.D3DCompiler",
        "Vortice.Mathematics",
        "SharpGen.Runtime",
        "SharpGen.Runtime.COM",
    };

    private static int _configured;

    public static void Configure()
    {
        if (Interlocked.Exchange(ref _configured, 1) != 0)
        {
            return;
        }

        AssemblyLoadContext.Default.Resolving += Resolve;
    }

    public static string GetCandidatePath(string baseDirectory, string assemblyName)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(baseDirectory);
        ArgumentException.ThrowIfNullOrWhiteSpace(assemblyName);
        if (!SupportedAssemblies.Contains(assemblyName))
        {
            throw new ArgumentException("Assembly is not an allowed external runtime dependency.", nameof(assemblyName));
        }

        var runtimeDirectory = assemblyName switch
        {
            "Microsoft.Windows.SDK.NET" or "WinRT.Runtime" => "winrt",
            _ => "gpu",
        };
        return Path.Combine(baseDirectory, "runtime", runtimeDirectory, assemblyName + ".dll");
    }

    private static Assembly? Resolve(AssemblyLoadContext context, AssemblyName assemblyName)
    {
        var simpleName = assemblyName.Name;
        if (string.IsNullOrWhiteSpace(simpleName) || !SupportedAssemblies.Contains(simpleName))
        {
            return null;
        }

        var candidate = GetCandidatePath(AppContext.BaseDirectory, simpleName);
        if (!File.Exists(candidate))
        {
            return null;
        }

        try
        {
            return context.LoadFromAssemblyPath(candidate);
        }
        catch (BadImageFormatException)
        {
            return null;
        }
        catch (FileLoadException)
        {
            return null;
        }
        catch (IOException)
        {
            return null;
        }
    }
}
