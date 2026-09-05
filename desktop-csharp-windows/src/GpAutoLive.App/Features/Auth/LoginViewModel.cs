using System.ComponentModel;
using System.Runtime.CompilerServices;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Features.Auth;

/// <summary>
/// C2 登录门禁的 WPF 投影。
///
/// 默认构造仍保持无网络的预览壳；传入控制面编排器后，登录/刷新/退出由唯一认证所有者执行。
/// 视图只保留短期密码输入，不持有 Refresh Token。
/// </summary>
public sealed class LoginViewModel : INotifyPropertyChanged
{
    private readonly ControlPlaneAuthCoordinator? _auth;
    private string _account = string.Empty;
    private string _password = string.Empty;
    private LoginStatus _status = LoginStatus.SignedOut;
    private string _message = "请输入账号和密码；离线续播仅适用于已授权会话";

    public LoginViewModel(ControlPlaneAuthCoordinator? auth = null)
    {
        _auth = auth;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public string Account
    {
        get => _account;
        set
        {
            var normalized = value ?? string.Empty;
            if (_account == normalized)
            {
                return;
            }

            _account = normalized;
            OnPropertyChanged();
            OnPropertyChanged(nameof(CanSubmit));
        }
    }

    /// <summary>
    /// 仅由 PasswordBox 在内存中更新，不提供可绑定的密码 getter。
    /// </summary>
    public void SetPassword(string password)
    {
        var normalized = password ?? string.Empty;
        if (_password == normalized)
        {
            return;
        }

        _password = normalized;
        OnPropertyChanged(nameof(HasPassword));
        OnPropertyChanged(nameof(CanSubmit));
    }

    public LoginStatus Status
    {
        get => _status;
        private set
        {
            if (_status == value)
            {
                return;
            }

            _status = value;
            OnPropertyChanged();
            OnPropertyChanged(nameof(StatusLabel));
            OnPropertyChanged(nameof(IsBusy));
            OnPropertyChanged(nameof(CanSubmit));
            OnPropertyChanged(nameof(CanRefresh));
            OnPropertyChanged(nameof(CanUseOffline));
            OnPropertyChanged(nameof(CanLogout));
            OnPropertyChanged(nameof(CanEnterWorkbench));
            OnPropertyChanged(nameof(IsGateVisible));
        }
    }

    public string Message
    {
        get => _message;
        private set
        {
            if (_message == value)
            {
                return;
            }

            _message = value;
            OnPropertyChanged();
        }
    }

    public bool HasPassword => _password.Length > 0;

    public bool IsBusy => Status is LoginStatus.SigningIn or LoginStatus.Refreshing;

    public bool CanSubmit => !IsBusy
        && Account.Trim().Length is >= AuthInputLimits.UsernameMinLength and <= AuthInputLimits.UsernameMaxLength
        && _password.Length is >= AuthInputLimits.PasswordMinLength and <= AuthInputLimits.PasswordMaxLength;

    public bool CanRefresh => !IsBusy && (Status is LoginStatus.Authenticated or LoginStatus.Activated);

    /// <summary>
    /// 只有已经通过控制面授权的会话才能转入离线续播；登录页不能用离线模式绕过授权。
    /// </summary>
    public bool CanUseOffline => !IsBusy && Status is LoginStatus.Activated;

    public bool CanLogout => Status is LoginStatus.Authenticated or LoginStatus.Activated or LoginStatus.Offline;

    /// <summary>仅允许已获服务端授权的会话进入工作台；Offline 只表示已进入工作台后的续播状态。</summary>
    public bool CanEnterWorkbench => Status is LoginStatus.Activated or LoginStatus.Offline;

    public bool IsGateVisible => !CanEnterWorkbench;

    public string StatusLabel => Status switch
    {
        LoginStatus.SignedOut => "未登录",
        LoginStatus.SigningIn => "登录中…",
        LoginStatus.Refreshing => "刷新中…",
        LoginStatus.Authenticated => "已登录",
        LoginStatus.Activated => "已授权",
        LoginStatus.Offline => "离线模式",
        LoginStatus.ActivationRequired => "需要激活",
        LoginStatus.Disabled => "账号已禁用",
        LoginStatus.Error => "登录不可用",
        _ => "未知状态",
    };

    /// <summary>
    /// 启动登录状态机；没有控制面配置时明确提示配置要求且不发网络请求。
    /// </summary>
    public async Task LoginAsync(CancellationToken cancellationToken = default)
    {
        if (!ValidateCredentials())
        {
            ClearPassword();
            return;
        }

        if (_auth is not null)
        {
            try
            {
                var transition = await _auth.LoginAsync(Account.Trim(), _password, cancellationToken).ConfigureAwait(true);
                ApplyTransition(transition);
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                Status = LoginStatus.SignedOut;
                Message = "登录已取消；当前未建立授权会话。";
            }
            catch (Exception)
            {
                Status = LoginStatus.Error;
                Message = "控制面登录未完成，请重试。";
            }
            finally
            {
                ClearPassword();
            }
            return;
        }

        Status = LoginStatus.SigningIn;
        Message = "正在准备登录…";

        try
        {
            // 让 WPF 有机会先呈现加载态；这里不是网络等待，也不伪造服务端成功。
            await Task.Delay(TimeSpan.FromMilliseconds(25), cancellationToken).ConfigureAwait(true);
            Status = LoginStatus.Error;
            Message = "控制面未配置；请设置 AUTOLIVE_CONTROL_PLANE_BASE_URI。直接双击 EXE 不会读取 launchSettings.json，开发联调请使用 tools\\start-csharp-development.cmd，正式环境需配置 HTTPS 地址；当前未发出网络请求。";
        }
        catch (OperationCanceledException)
        {
            Status = LoginStatus.SignedOut;
            Message = "登录已取消；未发出网络请求。";
        }
        finally
        {
            ClearPassword();
        }
    }

    /// <summary>刷新当前控制面会话；没有控制面配置时明确提示配置要求。</summary>
    public async Task RefreshAsync(CancellationToken cancellationToken = default)
    {
        if (!CanRefresh)
        {
            return;
        }

        if (_auth is not null)
        {
            try
            {
                var transition = await _auth.RefreshAsync(cancellationToken).ConfigureAwait(true);
                ApplyTransition(transition);
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                Status = LoginStatus.SignedOut;
                Message = "刷新已取消；当前未建立授权会话。";
            }
            catch (Exception)
            {
                Status = LoginStatus.Error;
                Message = "控制面刷新未完成，请重试。";
            }

            return;
        }

        ClearPassword();
        Status = LoginStatus.Refreshing;
        Message = "正在准备刷新会话…";

        try
        {
            await Task.Delay(TimeSpan.FromMilliseconds(25), cancellationToken).ConfigureAwait(true);
            Status = LoginStatus.Error;
            Message = "控制面未配置；请设置 AUTOLIVE_CONTROL_PLANE_BASE_URI，无法刷新会话，当前未发出网络请求。";
        }
        catch (OperationCanceledException)
        {
            Status = LoginStatus.SignedOut;
            Message = "刷新已取消；当前仍未登录。";
        }
    }

    public void UseOffline()
    {
        if (!CanUseOffline)
        {
            ClearPassword();
            Message = Status is LoginStatus.Disabled or LoginStatus.ActivationRequired or LoginStatus.Authenticated
                ? Message
                : "尚未完成设备授权，离线模式不能绕过登录或激活。";
            return;
        }

        ClearPassword();
        Status = LoginStatus.Offline;
        Message = "离线模式已启用；云端登录、激活与心跳不可用。";
    }

    /// <summary>
    /// 由控制面适配器投影已登录但尚未完成设备激活的会话。
    /// 登录按钮不会调用此方法，也不在本地伪造成功。
    /// </summary>
    public void ApplyAuthenticated(string account)
    {
        var normalized = account?.Trim() ?? string.Empty;
        if (normalized.Length == 0 || normalized.Length > 128)
        {
            ApplyError("控制面返回的账号摘要无效。登录仍未完成。");
            return;
        }

        Account = normalized;
        ClearPassword();
        Status = LoginStatus.Authenticated;
        Message = "已登录；设备尚未激活，完成激活后才能进入工作台。";
    }

    /// <summary>
    /// 由控制面适配器投影已完成设备激活的会话。只有此状态允许进入工作台或断网续播。
    /// </summary>
    public void ApplyActivated(string account)
    {
        var normalized = account?.Trim() ?? string.Empty;
        if (normalized.Length == 0 || normalized.Length > 128)
        {
            ApplyError("控制面返回的账号摘要无效。设备授权仍未完成。");
            return;
        }

        Account = normalized;
        ClearPassword();
        Status = LoginStatus.Activated;
        Message = "设备已激活，可以进入工作台。";
    }

    /// <summary>由控制面适配器投影稳定错误，不改变本地授权边界。</summary>
    public void ApplyError(string message)
    {
        var normalized = message?.Trim() ?? string.Empty;
        Status = LoginStatus.Error;
        Message = normalized.Length == 0 ? "控制面暂不可用；登录仍未完成。" : normalized;
    }

    public void Logout()
    {
        Account = string.Empty;
        ClearPassword();
        Status = LoginStatus.SignedOut;
        Message = "已退出登录；离线续播需先完成服务端授权。";
    }

    /// <summary>清理本地状态并尽力撤销远端会话。</summary>
    public async Task LogoutAsync(CancellationToken cancellationToken = default)
    {
        if (_auth is null)
        {
            Logout();
            return;
        }

        try
        {
            var transition = await _auth.LogoutAsync(cancellationToken).ConfigureAwait(true);
            ApplyTransition(transition);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            // 窗口关闭时不再更新已销毁的登录投影。
        }
        catch (Exception)
        {
            Status = LoginStatus.Error;
            Message = "退出登录未完成，请重试。";
        }
    }

    /// <summary>将控制面恢复/登录/刷新结果投影到 UI；ViewModel 不读取或保存 Token。</summary>
    public void ApplyTransition(AuthTransition transition)
    {
        ArgumentNullException.ThrowIfNull(transition);
        ApplySnapshot(transition.Snapshot);
        if (transition.Error is not null)
        {
            Message = transition.Error.Message;
        }
    }

    /// <summary>供控制面适配器投影账号禁用结果；当前 UI 不自行调用。</summary>
    public void ApplyDisabled(string? reason = null)
    {
        ClearPassword();
        Status = LoginStatus.Disabled;
        Message = string.IsNullOrWhiteSpace(reason) ? "账号已被禁用，请联系管理员。" : reason.Trim();
    }

    /// <summary>供控制面适配器投影设备激活门禁；当前 UI 不自行调用。</summary>
    public void ApplyActivationRequired(string? reason = null)
    {
        ClearPassword();
        Status = LoginStatus.ActivationRequired;
        Message = string.IsNullOrWhiteSpace(reason) ? "设备尚未激活，请完成激活后再继续。" : reason.Trim();
    }

    private void ApplySnapshot(AuthSessionSnapshot snapshot)
    {
        Status = snapshot.State switch
        {
            AuthSessionState.Authenticating => LoginStatus.SigningIn,
            AuthSessionState.Authenticated => LoginStatus.Authenticated,
            AuthSessionState.Activated => LoginStatus.Activated,
            AuthSessionState.Offline => LoginStatus.Offline,
            AuthSessionState.Disabled => LoginStatus.Disabled,
            AuthSessionState.SigningOut => LoginStatus.Refreshing,
            _ => LoginStatus.SignedOut,
        };

        Message = snapshot.LastError?.Message ?? Status switch
        {
            LoginStatus.Authenticated => "已登录；设备尚未激活，完成激活后才能进入工作台。",
            LoginStatus.Activated => "设备已激活，可以进入工作台。",
            LoginStatus.Offline => "离线模式已启用；云端登录、激活与心跳不可用。",
            LoginStatus.Disabled => "账号或设备已被禁用，请联系管理员。",
            _ => "控制面会话尚未建立。",
        };
    }

    private bool ValidateCredentials()
    {
        var accountLength = Account.Trim().Length;
        if (accountLength is < AuthInputLimits.UsernameMinLength or > AuthInputLimits.UsernameMaxLength)
        {
            Status = LoginStatus.Error;
            Message = $"账号长度需为 {AuthInputLimits.UsernameMinLength}～{AuthInputLimits.UsernameMaxLength} 个字符。";
            return false;
        }

        if (_password.Length < AuthInputLimits.PasswordMinLength)
        {
            Status = LoginStatus.Error;
            Message = $"密码长度至少为 {AuthInputLimits.PasswordMinLength} 个字符；密码只保存在当前进程内。";
            return false;
        }

        if (_password.Length > AuthInputLimits.PasswordMaxLength)
        {
            Status = LoginStatus.Error;
            Message = $"密码长度不能超过 {AuthInputLimits.PasswordMaxLength} 个字符。";
            return false;
        }

        return true;
    }

    private void ClearPassword()
    {
        if (_password.Length == 0)
        {
            return;
        }

        _password = string.Empty;
        OnPropertyChanged(nameof(HasPassword));
        OnPropertyChanged(nameof(CanSubmit));
    }

    private void OnPropertyChanged([CallerMemberName] string? propertyName = null) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
}
