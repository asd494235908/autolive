using GpAutoLive.App.Features.Auth;

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
        Assert.IsFalse(viewModel.CanEnterWorkbench);
        Assert.IsFalse(viewModel.CanRefresh);
        Assert.IsFalse(viewModel.CanUseOffline);
        viewModel.UseOffline();
        Assert.AreEqual(LoginStatus.Disabled, viewModel.Status);
        Assert.IsFalse(viewModel.CanEnterWorkbench);
    }
}
