using System.IO;
using System.Windows.Media.Imaging;

namespace GpAutoLive.App.Features.Douyin;

/// <summary>
/// 受限加载本地抖音二维码图片，成功后不再持有源文件句柄。
/// </summary>
public static class DouyinQrImageLoader
{
    private const long MaxPngBytes = 16 * 1024 * 1024;

    public static bool TryLoad(string? path, out BitmapImage? image, out string? error)
    {
        image = null;
        error = null;

        if (string.IsNullOrWhiteSpace(path))
        {
            error = "二维码图片路径为空。";
            return false;
        }

        try
        {
            var fullPath = Path.GetFullPath(path);
            if (Directory.Exists(fullPath))
            {
                error = "二维码图片路径不是文件。";
                return false;
            }

            if (!string.Equals(Path.GetExtension(fullPath), ".png", StringComparison.OrdinalIgnoreCase))
            {
                error = "二维码图片必须是 PNG 文件。";
                return false;
            }

            if (!File.Exists(fullPath))
            {
                error = "二维码图片文件不存在。";
                return false;
            }

            var attributes = File.GetAttributes(fullPath);
            if ((attributes & FileAttributes.ReparsePoint) != 0)
            {
                error = "二维码图片不支持重解析点。";
                return false;
            }

            var fileInfo = new FileInfo(fullPath);
            if (fileInfo.Length > MaxPngBytes)
            {
                error = "二维码图片文件过大。";
                return false;
            }

            using var stream = new FileStream(
                fullPath,
                FileMode.Open,
                FileAccess.Read,
                FileShare.Read,
                bufferSize: 4096,
                options: FileOptions.SequentialScan);
            var bitmap = new BitmapImage();
            bitmap.BeginInit();
            bitmap.CacheOption = BitmapCacheOption.OnLoad;
            bitmap.StreamSource = stream;
            bitmap.EndInit();
            bitmap.Freeze();

            image = bitmap;
            return true;
        }
        catch (ArgumentException)
        {
            error = "二维码图片路径或内容无效。";
        }
        catch (UnauthorizedAccessException)
        {
            error = "二维码图片无权读取。";
        }
        catch (System.Security.SecurityException)
        {
            error = "二维码图片访问被拒绝。";
        }
        catch (IOException)
        {
            error = "二维码图片无法读取。";
        }
        catch (InvalidOperationException)
        {
            error = "二维码图片内容无效。";
        }
        catch (NotSupportedException)
        {
            error = "二维码图片格式不受支持。";
        }

        return false;
    }
}
