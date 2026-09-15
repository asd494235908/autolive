namespace GpAutoLive.Windows;

/// <summary>仅说明本次运行内为什么没有可复用的登录；登录事实仍由 Authenticated 表达。</summary>
public enum WindowsDouyinLoginClearReason
{
    /// <summary>本次软件运行尚未确认抖音登录，包括重新启动后。</summary>
    NotAuthenticatedThisRun,
    /// <summary>登录仍有效，没有清除原因。</summary>
    None,
    /// <summary>平台明确报告已有登录已失效。</summary>
    AuthenticationExpired,
    /// <summary>扫码登录失败或二维码等待超时。</summary>
    LoginFailed,
    /// <summary>用户取消、完整停止或退出软件。</summary>
    ExplicitStop,
    /// <summary>受管辅助进程已退出或无法启动。</summary>
    ProcessExited,
    /// <summary>本地进程通信异常，无法安全确认会话后清理。</summary>
    LocalCommunicationError
}
