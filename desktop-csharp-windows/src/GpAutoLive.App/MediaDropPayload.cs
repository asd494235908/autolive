using System.Runtime.InteropServices;
using System.Windows;
using GpAutoLive.Contracts;
using GpAutoLive.Media;

namespace GpAutoLive.App;

/// <summary>
/// 读取 Windows 原生 FileDrop 数据。拖放内容只作为候选路径交给媒体导入协调器，
/// 不在 UI 层探测、去重或直接写入播放池。
/// </summary>
public static class MediaDropPayload
{
    /// <summary>快速判断拖放数据是否包含数量合法且扩展名受支持的 FileDrop 候选，不复制路径数组。</summary>
    public static bool HasCandidateFiles(IDataObject? data)
    {
        if (!TryGetRawPaths(data, out var paths)
            || paths.Length == 0
            || paths.Length > MediaPoolRules.MaxItems)
        {
            return false;
        }

        for (var index = 0; index < paths.Length; index++)
        {
            if (string.IsNullOrWhiteSpace(paths[index])
                || !MediaFormatCatalog.IsSupportedExtension(paths[index]))
            {
                return false;
            }
        }

        return true;
    }

    /// <summary>
    /// 复制并校验原生 FileDrop 路径，保留操作系统返回顺序。
    /// </summary>
    public static bool TryReadPaths(IDataObject? data, out string[] paths)
    {
        paths = [];
        if (!TryGetRawPaths(data, out var rawPaths)
            || rawPaths.Length == 0
            || rawPaths.Length > MediaPoolRules.MaxItems)
        {
            return false;
        }

        for (var index = 0; index < rawPaths.Length; index++)
        {
            if (string.IsNullOrWhiteSpace(rawPaths[index])
                || !MediaFormatCatalog.IsSupportedExtension(rawPaths[index]))
            {
                return false;
            }
        }

        paths = rawPaths.ToArray();
        return true;
    }

    private static bool TryGetRawPaths(IDataObject? data, out string[] paths)
    {
        paths = [];
        if (data is null)
        {
            return false;
        }

        try
        {
            if (!data.GetDataPresent(DataFormats.FileDrop, autoConvert: false)
                || data.GetData(DataFormats.FileDrop, autoConvert: false) is not string[] rawPaths)
            {
                return false;
            }

            paths = rawPaths;
            return true;
        }
        catch (Exception exception) when (
            exception is ArgumentException
            or InvalidOperationException
            or NotSupportedException
            or ExternalException)
        {
            // OLE drag sources are outside the process; malformed data is simply rejected.
            return false;
        }
    }
}
