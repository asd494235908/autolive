using System.Windows;
using System.Windows.Controls;

namespace GpAutoLive.App.Features.Auth;

public partial class LoginView : UserControl
{
    public LoginView()
    {
        InitializeComponent();
    }

    private LoginViewModel? ViewModel => DataContext as LoginViewModel;

    private void LoginView_Loaded(object sender, RoutedEventArgs e)
    {
        AccountBox.Focus();
    }

    private void PasswordBox_PasswordChanged(object sender, RoutedEventArgs e)
    {
        ViewModel?.SetPassword(PasswordBox.Password);
    }

    private async void LoginButton_Click(object sender, RoutedEventArgs e)
    {
        var viewModel = ViewModel;
        if (viewModel is not null)
        {
            try
            {
                await viewModel.LoginAsync();
            }
            finally
            {
                // 登录尝试完成后清除控件副本，避免密码因重试界面长期驻留内存。
                PasswordBox.Clear();
            }
        }
    }

    private async void RefreshButton_Click(object sender, RoutedEventArgs e)
    {
        var viewModel = ViewModel;
        if (viewModel is not null)
        {
            await viewModel.RefreshAsync();
        }
    }

    private void OfflineButton_Click(object sender, RoutedEventArgs e) => ViewModel?.UseOffline();

    private void ActivationButton_Click(object sender, RoutedEventArgs e) => ViewModel?.ApplyActivationRequired();

    /// <summary>状态离开输入阶段时同步清空控件内部的密码副本。</summary>
    public void ClearPasswordInput() => PasswordBox.Clear();
}
