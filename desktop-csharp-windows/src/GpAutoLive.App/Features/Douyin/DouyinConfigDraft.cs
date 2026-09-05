using System.Collections.Immutable;
using System.Globalization;
using GpAutoLive.Contracts;

namespace GpAutoLive.App.Features.Douyin;

/// <summary>把 WPF 编辑态转换为抖音 M1 本地配置；不保存凭据或平台会话。</summary>
public static class DouyinConfigDraft
{
    public static bool TryCreate(
        bool enabled,
        string? roomId,
        string? repliesText,
        string? queueCapacityText,
        out DouyinLiveConfig? config,
        out string? error)
    {
        config = null;
        error = null;
        if (!TryParseQueueCapacity(queueCapacityText, out var queueCapacity, out error))
        {
            return false;
        }

        var replies = (repliesText ?? string.Empty)
            .Split(
                ["\r\n", "\n", "\r"],
                StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .ToImmutableArray();
        var candidate = new DouyinLiveConfig
        {
            Enabled = enabled,
            RoomId = roomId ?? string.Empty,
            Replies = replies,
            QueueCapacity = queueCapacity,
        };

        if (!DouyinLiveRules.TryNormalize(candidate, out config, out var validationError))
        {
            error = validationError?.Message ?? "抖音 M1 配置无效。";
            return false;
        }

        return true;
    }

    public static bool TryParseQueueCapacity(
        string? value,
        out int capacity,
        out string? error)
    {
        error = null;
        if (int.TryParse(
                value?.Trim(),
                NumberStyles.Integer,
                CultureInfo.InvariantCulture,
                out capacity))
        {
            return true;
        }

        capacity = 0;
        error = "队列容量必须是整数。";
        return false;
    }
}
