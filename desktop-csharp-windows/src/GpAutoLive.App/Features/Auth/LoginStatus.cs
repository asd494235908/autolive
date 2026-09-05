namespace GpAutoLive.App.Features.Auth;

/// <summary>
/// 登录门禁的 UI 投影状态。它描述当前客户端能做什么，不代表服务端已完成真实接入。
/// </summary>
public enum LoginStatus
{
    SignedOut,
    SigningIn,
    Refreshing,
    Authenticated,
    Activated,
    Offline,
    ActivationRequired,
    Disabled,
    Error,
}
