using System.Runtime.InteropServices;

namespace GpAutoLive.Windows;

/// <summary>当前 C# 客户端的 Windows shell 身份配置结果。</summary>
public enum WindowsAppIdentityCode
{
    Applied,
    NotWindows,
    InvalidAppUserModelId,
    ApiUnavailable,
    ApiFailed,
}

/// <summary>
/// 设置独立的 AppUserModelID，避免 C# 客户端与 Rust/Tauri 安装入口共享任务栏身份。
/// </summary>
public static class WindowsAppIdentity
{
    /// <summary>与 Rust/Tauri 参考实现隔离的 C# Windows shell 身份。</summary>
    public const string AppUserModelId = "GpAutoLive.CSharp.Windows";

    /// <summary>只调用 Windows Shell 原生 API；失败时返回分类结果，不阻断应用启动。</summary>
    public static WindowsAppIdentityCode TryConfigure()
    {
        if (!OperatingSystem.IsWindows())
        {
            return WindowsAppIdentityCode.NotWindows;
        }

        if (string.IsNullOrWhiteSpace(AppUserModelId)
            || AppUserModelId.Length > 128
            || AppUserModelId.Any(char.IsControl))
        {
            return WindowsAppIdentityCode.InvalidAppUserModelId;
        }

        try
        {
            return SetCurrentProcessExplicitAppUserModelID(AppUserModelId) == 0
                ? WindowsAppIdentityCode.Applied
                : WindowsAppIdentityCode.ApiFailed;
        }
        catch (DllNotFoundException)
        {
            return WindowsAppIdentityCode.ApiUnavailable;
        }
        catch (EntryPointNotFoundException)
        {
            return WindowsAppIdentityCode.ApiUnavailable;
        }
        catch (BadImageFormatException)
        {
            return WindowsAppIdentityCode.ApiUnavailable;
        }
    }

    [DllImport("shell32.dll", ExactSpelling = true, CharSet = CharSet.Unicode)]
    private static extern int SetCurrentProcessExplicitAppUserModelID(
        [MarshalAs(UnmanagedType.LPWStr)] string appUserModelId);
}
