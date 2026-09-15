using System.Collections.Immutable;
using System.Collections.ObjectModel;
using System.Windows;
using System.Windows.Controls;
using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Features.Douyin;

/// <summary>当前会话弹幕的内存显示投影；关闭窗口不改变宿主监听生命周期。</summary>
public partial class DouyinChatWindow : Window
{
    private readonly Action _clearMessages;
    private readonly ObservableCollection<DouyinChatDisplayMessage> _messages = [];
    private bool _followLatest = true;
    private readonly Func<string, Task<WindowsDouyinChatSendResult?>>? _sendChat;
    private bool _canSend;
    private bool _hostSending;
    private bool _sendPending;

    public DouyinChatWindow(Action clearMessages, Func<string, Task<WindowsDouyinChatSendResult?>>? sendChat = null)
    {
        _clearMessages = clearMessages;
        _sendChat = sendChat;
        InitializeComponent();
        MessagesList.ItemsSource = _messages;
        ManualChatTextBox.MaxLength = DouyinLiveRules.MaxReplyCharacters * 2;
    }

    public void RefreshSendState(bool canSend, bool sending, string? message)
    {
        _canSend = canSend;
        _hostSending = sending;
        ManualSendStatusText.Text = sending ? "正在发送，请勿重复点击"
            : message ?? (canSend ? "最多 80 个字符；Enter 不会发送" : "连接直播间并恢复监听后可手动发送");
        UpdateSendControls();
    }

    private void UpdateSendControls()
    {
        var busy = _sendPending || _hostSending;
        ManualChatTextBox.IsEnabled = !busy;
        SendChatButton.IsEnabled = _sendChat is not null && _canSend && !busy
            && !string.IsNullOrWhiteSpace(ManualChatTextBox.Text);
    }

    private void ManualChatTextBox_TextChanged(object sender, TextChangedEventArgs e)
    {
        if (SendChatButton is not null) UpdateSendControls();
    }

    private async void SendChatButton_Click(object sender, RoutedEventArgs e)
    {
        if (!SendChatButton.IsEnabled || _sendChat is null) return;
        var draft = ManualChatTextBox.Text;
        _sendPending = true;
        ManualSendStatusText.Text = "正在发送，请勿重复点击";
        UpdateSendControls();
        try
        {
            // 主窗口拥有发送任务和结果；关闭本窗口不会取消发送或更新另一个窗口实例。
            var result = await _sendChat(draft.Trim()).ConfigureAwait(true);
            if (result?.Outcome == DouyinSendOutcome.Accepted && ManualChatTextBox.Text == draft)
                ManualChatTextBox.Clear();
        }
        finally
        {
            _sendPending = false;
            UpdateSendControls();
        }
    }

    public void Refresh(ImmutableArray<DouyinChatDisplayMessage> messages, string status, string? roomId, string? statusDetails = null)
    {
        Title = string.IsNullOrWhiteSpace(roomId) ? "实时弹幕" : $"实时弹幕 · 房间 {roomId}";
        ConnectionStatusText.Text = status;
        ConnectionStatusText.ToolTip = statusDetails ?? status;
        if (messages.IsEmpty)
        {
            _messages.Clear();
            _followLatest = true;
        }
        else
        {
            // 保留交集中的行，避免每次状态刷新重建列表打断正在阅读的位置。
            while (_messages.Count > 0 && _messages[0] != messages[0]) _messages.RemoveAt(0);
            for (var index = _messages.Count; index < messages.Length; index++) _messages.Add(messages[index]);
        }
        EmptyText.Visibility = _messages.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        MessageCountText.Text = $"最近 {_messages.Count} / 500 条";
        if (_followLatest && _messages.Count > 0) MessagesList.ScrollIntoView(_messages[^1]);
    }

    private void MessagesList_ScrollChanged(object sender, ScrollChangedEventArgs e)
    {
        // 内容增减也触发 ScrollChanged，只有用户移动视口才改变跟随状态。
        if (e.ExtentHeightChange == 0 && e.ViewportHeightChange == 0 && e.VerticalChange != 0)
            _followLatest = e.VerticalOffset + e.ViewportHeight >= e.ExtentHeight - 1;
    }

    private void LatestButton_Click(object sender, RoutedEventArgs e)
    {
        _followLatest = true;
        if (_messages.Count > 0) MessagesList.ScrollIntoView(_messages[^1]);
    }

    private void ClearButton_Click(object sender, RoutedEventArgs e)
    {
        _clearMessages();
        _messages.Clear();
        _followLatest = true;
        EmptyText.Visibility = Visibility.Visible;
        MessageCountText.Text = "最近 0 / 500 条";
    }
}
