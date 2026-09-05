namespace GpAutoLive.Windows;

/// <summary>
/// Windows-only integration boundary for HWND, process lifetime, credentials,
/// DPI, media playback and capture adapters. 真实设备、下游兼容、签名和长稳
/// 仍由各专项门禁单独验收，不在这里把代码接入冒充为发布完成。
/// </summary>
public static class WindowsCapabilityBoundary
{
    public const string Target = "Windows 10/11 x64";
    public const string Status = "代码已接入·待验收";
}
