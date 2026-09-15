using System.IO;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using GpAutoLive.App.Features.Playback;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class ReservedVideoSurfaceTests
{
    [TestMethod]
    public void Held_frame_survives_invalid_replacement_resize_and_releases_on_close()
    {
        WpfTestApplicationHost.Run(() =>
        {
            var surface = new ReservedVideoSurface();
            var window = new Window { Content = surface, Width = 240, Height = 160, ShowActivated = false };
            try
            {
                window.Show();
                window.UpdateLayout();
                var source = BitmapSource.Create(2, 2, 96, 96, PixelFormats.Bgra32, null,
                    new byte[] { 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255 }, 8);
                var encoder = new PngBitmapEncoder();
                encoder.Frames.Add(BitmapFrame.Create(source));
                using var stream = new MemoryStream();
                encoder.Save(stream);
                Assert.IsTrue(surface.TryHoldFrame(stream.ToArray(), out var error), error);
                Assert.IsTrue(surface.HasHeldFrame);
                Assert.IsFalse(surface.TryHoldFrame([1, 2, 3], out error));
                Assert.IsNotNull(error);
                Assert.IsTrue(surface.HasHeldFrame, "失败的替换不能撤掉既有画面。");
                Assert.IsFalse(surface.TryHoldFrame(stream.ToArray()[..33], out error));
                Assert.IsTrue(surface.HasHeldFrame, "PNG 解码失败也必须保持已有画面。");
                var oversized = stream.ToArray();
                oversized[16] = 0x7f;
                Assert.IsFalse(surface.TryHoldFrame(oversized, out error));
                Assert.IsTrue(surface.HasHeldFrame);
                System.Buffers.Binary.BinaryPrimitives.WriteUInt32BigEndian(oversized.AsSpan(16, 4), 8192);
                System.Buffers.Binary.BinaryPrimitives.WriteUInt32BigEndian(oversized.AsSpan(20, 4), 8192);
                Assert.IsFalse(surface.TryHoldFrame(oversized, out error));
                StringAssert.Contains(error, "总像素");
                Assert.IsTrue(surface.HasHeldFrame);
                window.Width = 420;
                window.UpdateLayout();
                Assert.IsTrue(surface.HasHeldFrame);
                surface.ReleaseHeldFrame();
                Assert.IsFalse(surface.HasHeldFrame);
                Assert.IsTrue(surface.TryHoldFrame(stream.ToArray(), out error), error);
            }
            finally { window.Close(); }
            Assert.IsFalse(surface.HasHeldFrame);
        });
    }
}
