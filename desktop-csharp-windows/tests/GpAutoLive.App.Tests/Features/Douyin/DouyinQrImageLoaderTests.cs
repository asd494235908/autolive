using System.IO;
using GpAutoLive.App.Features.Douyin;

namespace GpAutoLive.App.Tests.Features.Douyin;

[TestClass]
public sealed class DouyinQrImageLoaderTests
{
    private static readonly byte[] OnePixelPng =
    [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
        0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41,
        0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
        0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99,
        0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
        0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82
    ];

    [TestMethod]
    public void Valid_png_is_loaded_frozen_and_does_not_lock_file()
    {
        using var fixture = TempFile.Create(".png", OnePixelPng);

        Assert.IsTrue(DouyinQrImageLoader.TryLoad(fixture.Path, out var image, out var error), error);
        Assert.IsNotNull(image);
        Assert.IsTrue(image!.IsFrozen);

        File.Delete(fixture.Path);
        Assert.IsFalse(File.Exists(fixture.Path));
    }

    [TestMethod]
    [DataRow(null)]
    [DataRow("")]
    [DataRow("   ")]
    public void Empty_path_fails_without_path_in_error(string? path)
    {
        Assert.IsFalse(DouyinQrImageLoader.TryLoad(path, out var image, out var error));
        Assert.IsNull(image);
        Assert.IsFalse(string.IsNullOrWhiteSpace(error));
    }

    [TestMethod]
    public void Missing_directory_and_non_png_paths_fail()
    {
        using var fixture = TempFile.Create(".jpg", [0x01]);
        var missing = System.IO.Path.Combine(fixture.DirectoryPath, "missing.png");

        AssertFailure(missing, "不存在");
        AssertFailure(fixture.DirectoryPath, "不是文件");
        AssertFailure(fixture.Path, "PNG");
    }

    [TestMethod]
    public void Oversized_png_fails_before_image_decode()
    {
        using var fixture = TempFile.Create(".png", OnePixelPng);
        using (var stream = new FileStream(fixture.Path, FileMode.Open, FileAccess.Write, FileShare.Read))
        {
            stream.SetLength(16 * 1024 * 1024 + 1);
        }

        AssertFailure(fixture.Path, "过大");
    }

    [TestMethod]
    public void Reparse_point_is_rejected()
    {
        using var fixture = TempFile.Create(".png", OnePixelPng);
        var linkPath = System.IO.Path.Combine(fixture.DirectoryPath, "qr-link.png");
        try
        {
            File.CreateSymbolicLink(linkPath, fixture.Path);
        }
        catch (UnauthorizedAccessException)
        {
            Assert.Inconclusive("当前测试环境不允许创建 Windows 符号链接。");
            return;
        }
        catch (IOException)
        {
            Assert.Inconclusive("当前文件系统不支持创建 Windows 符号链接。");
            return;
        }
        catch (PlatformNotSupportedException)
        {
            Assert.Inconclusive("当前运行时不支持创建 Windows 符号链接。");
            return;
        }

        try
        {
            AssertFailure(linkPath, "重解析点");
        }
        finally
        {
            File.Delete(linkPath);
        }
    }

    private static void AssertFailure(string path, string expectedMessage)
    {
        Assert.IsFalse(DouyinQrImageLoader.TryLoad(path, out var image, out var error));
        Assert.IsNull(image);
        StringAssert.Contains(error!, expectedMessage);
        Assert.IsFalse(error!.Contains(path, StringComparison.Ordinal));
    }

    private sealed class TempFile : IDisposable
    {
        private TempFile(string path, string directoryPath)
        {
            Path = path;
            DirectoryPath = directoryPath;
        }

        public string Path { get; }

        public string DirectoryPath { get; }

        public static TempFile Create(string extension, byte[] bytes)
        {
            var directory = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "gpautolive-qr-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(directory);
            var path = System.IO.Path.Combine(directory, "qr" + extension);
            File.WriteAllBytes(path, bytes);
            return new TempFile(path, directory);
        }

        public void Dispose()
        {
            if (Directory.Exists(DirectoryPath))
            {
                Directory.Delete(DirectoryPath, recursive: true);
            }
        }
    }
}
