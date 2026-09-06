using GpAutoLive.App.Features.Auth;
using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class LoginViewModelTests
{
    [TestMethod]
    public void InitialStateKeepsWorkbenchBehindLoginGate()
    {
        var viewModel = new LoginViewModel();

        Assert.AreEqual(LoginStatus.SignedOut, viewModel.Status);
        Assert.IsTrue(viewModel.IsGateVisible);
        Assert.IsFalse(viewModel.CanEnterWorkbench);
        Assert.IsFalse(viewModel.CanSubmit);
        Assert.IsFalse(viewModel.CanLogout);
    }

    [TestMethod]
    public async Task LoginValidatesLocallyAndReportsNotConnected()
    {
        var viewModel = new LoginViewModel { Account = " demo@example.com " };
        viewModel.SetPassword("only-in-memory");
        var statuses = new List<LoginStatus>();
        viewModel.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName == nameof(LoginViewModel.Status))
            {
                statuses.Add(viewModel.Status);
            }
        };

        await viewModel.LoginAsync();

        Assert.AreEqual(LoginStatus.Error, viewModel.Status);
        CollectionAssert.Contains(statuses, LoginStatus.SigningIn);
        StringAssert.Contains(viewModel.Message, "AUTOLIVE_CONTROL_PLANE_BASE_URI");
        Assert.IsTrue(viewModel.IsGateVisible);
        Assert.IsFalse(viewModel.HasPassword);
    }

    [TestMethod]
    public async Task Login_rejects_credentials_outside_contract_limits()
    {
        var viewModel = new LoginViewModel { Account = "ab" };
        viewModel.SetPassword("short");

        await viewModel.LoginAsync();

        Assert.AreEqual(LoginStatus.Error, viewModel.Status);
        StringAssert.Contains(viewModel.Message, "长度");
        Assert.IsFalse(viewModel.HasPassword);
    }

    [TestMethod]
    public async Task RefreshIsAvailableOnlyForAuthorizedSessionAndKeepsFailClosed()
    {
        var viewModel = new LoginViewModel();
        Assert.IsFalse(viewModel.CanRefresh);

        viewModel.ApplyAuthenticated("demo@example.com");
        Assert.IsTrue(viewModel.CanRefresh);

        await viewModel.RefreshAsync();

        Assert.AreEqual(LoginStatus.Error, viewModel.Status);
        Assert.IsFalse(viewModel.CanEnterWorkbench);
        StringAssert.Contains(viewModel.Message, "刷新");
    }

    [TestMethod]
    public void UnauthorizedOfflineModeCannotBypassLoginGate()
    {
        var viewModel = new LoginViewModel();

        viewModel.UseOffline();

        Assert.AreEqual(LoginStatus.SignedOut, viewModel.Status);
        Assert.IsFalse(viewModel.CanEnterWorkbench);
        Assert.IsTrue(viewModel.IsGateVisible);
        Assert.IsFalse(viewModel.CanLogout);
        StringAssert.Contains(viewModel.Message, "不能绕过登录");
    }

    [TestMethod]
    public void AuthorizedSessionCanContinueOfflineAndLogoutRestoresGate()
    {
        var viewModel = new LoginViewModel();

        viewModel.ApplyAuthenticated("demo@example.com");
        Assert.IsFalse(viewModel.CanEnterWorkbench);
        Assert.IsFalse(viewModel.CanUseOffline);
        Assert.IsFalse(viewModel.IsCredentialEntryVisible);

        viewModel.ApplyActivated("demo@example.com");
        Assert.IsTrue(viewModel.CanEnterWorkbench);
        Assert.IsTrue(viewModel.CanUseOffline);

        viewModel.UseOffline();

        Assert.AreEqual(LoginStatus.Offline, viewModel.Status);
        Assert.IsTrue(viewModel.CanEnterWorkbench);
        Assert.IsFalse(viewModel.IsGateVisible);
        Assert.IsTrue(viewModel.CanLogout);
        Assert.IsFalse(viewModel.CanRefresh);
        StringAssert.Contains(viewModel.Message, "离线模式");

        viewModel.Logout();

        Assert.AreEqual(LoginStatus.SignedOut, viewModel.Status);
        Assert.IsFalse(viewModel.CanEnterWorkbench);
        Assert.IsTrue(viewModel.IsGateVisible);
        Assert.IsTrue(viewModel.IsCredentialEntryVisible);
        Assert.IsFalse(viewModel.CanLogout);
    }

    [TestMethod]
    public void DisabledAndActivationStatesRemainFailClosed()
    {
        var viewModel = new LoginViewModel();

        viewModel.ApplyActivationRequired();
        Assert.AreEqual(LoginStatus.ActivationRequired, viewModel.Status);
        Assert.IsFalse(viewModel.CanEnterWorkbench);
        Assert.IsFalse(viewModel.CanUseOffline);
        viewModel.UseOffline();
        Assert.AreEqual(LoginStatus.ActivationRequired, viewModel.Status);
        Assert.IsFalse(viewModel.CanEnterWorkbench);

        viewModel.ApplyDisabled();
        Assert.AreEqual(LoginStatus.Disabled, viewModel.Status);
        Assert.IsFalse(viewModel.IsCredentialEntryVisible);
        Assert.IsFalse(viewModel.CanEnterWorkbench);
        Assert.IsFalse(viewModel.CanRefresh);
        Assert.IsFalse(viewModel.CanUseOffline);
        viewModel.UseOffline();
        Assert.AreEqual(LoginStatus.Disabled, viewModel.Status);
        Assert.IsFalse(viewModel.CanEnterWorkbench);
    }

    [TestMethod]
    public void Transition_warning_is_visible_without_changing_signed_out_state()
    {
        var viewModel = new LoginViewModel();
        var transition = new AuthTransition(
            AuthTransitionKind.Completed,
            AuthOperationKind.Logout,
            "logout-warning",
            AuthSessionSnapshot.Initial)
        {
            Warning = "远端退出待重试。"
        };

        viewModel.ApplyTransition(transition);

        Assert.AreEqual(LoginStatus.SignedOut, viewModel.Status);
        Assert.AreEqual("远端退出待重试。", viewModel.Message);
    }

    [TestMethod]
    public void Login_error_is_identified_as_account_login_failure()
    {
        var viewModel = new LoginViewModel();
        var error = new ControlPlaneErrorDto(
            AuthErrorCodes.Unauthenticated,
            "账号或密码错误",
            401,
            "login-request");
        var transition = new AuthTransition(
            AuthTransitionKind.Rejected,
            AuthOperationKind.Login,
            "login-operation",
            AuthSessionSnapshot.Initial with { LastError = error },
            error);

        viewModel.ApplyTransition(transition);

        StringAssert.StartsWith(viewModel.Message, "账号登录失败");
        StringAssert.Contains(viewModel.Message, "UNAUTHENTICATED");
        StringAssert.Contains(viewModel.Message, "login-request");
    }

    [TestMethod]
    public void Activation_error_keeps_login_and_explains_authorization_retry()
    {
        var viewModel = new LoginViewModel();
        var error = new ControlPlaneErrorDto(
            AuthErrorCodes.InvalidRequest,
            "请求格式或参数无效",
            400,
            "activate-request");
        var transition = new AuthTransition(
            AuthTransitionKind.Rejected,
            AuthOperationKind.Activation,
            "activate-operation",
            new AuthSessionSnapshot
            {
                State = AuthSessionState.Authenticated,
                LastError = error
            },
            error);

        viewModel.ApplyTransition(transition);

        Assert.AreEqual(LoginStatus.Authenticated, viewModel.Status);
        StringAssert.StartsWith(viewModel.Message, "账号登录成功，但设备授权失败");
        StringAssert.Contains(viewModel.Message, "INVALID_ARGUMENT");
        StringAssert.Contains(viewModel.Message, "activate-request");
        StringAssert.Contains(viewModel.Message, "重试设备授权");
    }

    [TestMethod]
    public void Activation_unauthorized_requires_a_new_login()
    {
        var viewModel = new LoginViewModel();
        var error = new ControlPlaneErrorDto(
            AuthErrorCodes.Unauthenticated,
            "会话已失效",
            401,
            "activate-unauthorized");
        var transition = new AuthTransition(
            AuthTransitionKind.Rejected,
            AuthOperationKind.Activation,
            "activate-operation",
            AuthSessionSnapshot.Initial with { LastError = error },
            error);

        viewModel.ApplyTransition(transition);

        Assert.AreEqual(LoginStatus.SignedOut, viewModel.Status);
        StringAssert.StartsWith(viewModel.Message, "登录会话已失效");
        StringAssert.Contains(viewModel.Message, "请重新登录");
    }

    [TestMethod]
    public void Device_limit_error_is_explained_as_an_authorization_limit()
    {
        var viewModel = new LoginViewModel();
        var error = new ControlPlaneErrorDto(
            AuthErrorCodes.DeviceLimitExceeded,
            "设备数量已达到授权上限",
            409);
        var transition = new AuthTransition(
            AuthTransitionKind.Rejected,
            AuthOperationKind.Activation,
            "activate-operation",
            new AuthSessionSnapshot
            {
                State = AuthSessionState.Authenticated,
                LastError = error
            },
            error);

        viewModel.ApplyTransition(transition);

        StringAssert.Contains(viewModel.Message, "可登录设备数量已达到授权上限");
        StringAssert.Contains(viewModel.Message, "DEVICE_LIMIT_EXCEEDED");
        StringAssert.Contains(viewModel.Message, "重试设备授权");
    }
}
