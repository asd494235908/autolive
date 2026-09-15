using System.Text.Json;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsDouyinDiagnosticLogTests
{
    [TestMethod]
    public void Log_is_lazy_and_rotates_with_two_bounded_files()
    {
        var root = Path.Combine(Path.GetTempPath(), "gpautolive-douyin-log-tests", Guid.NewGuid().ToString("N"));
        try
        {
            var log = new WindowsDouyinDiagnosticLog(root, 1024);
            Assert.AreEqual(WindowsDouyinDiagnosticLogState.NotStarted, log.Snapshot.State);
            Assert.IsFalse(Directory.Exists(root));
            log.BeginRun();
            for (var i = 0; i < 30; i++) log.RecordDiagnostic(new("qr_fetch", "http_error", "http", 403, i));
            Assert.AreEqual(WindowsDouyinDiagnosticLogState.Ready, log.Snapshot.State);
            var files = Directory.GetFiles(root);
            Assert.AreEqual(2, files.Length);
            foreach (var file in files)
            {
                Assert.IsTrue(new FileInfo(file).Length <= 1024);
                foreach (var line in File.ReadLines(file))
                {
                    using var entry = JsonDocument.Parse(line);
                    Assert.AreEqual(32, entry.RootElement.GetProperty("runId").GetString()!.Length);
                    Assert.IsTrue(entry.RootElement.TryGetProperty("timestamp", out _));
                }
            }
        }
        finally { if (Directory.Exists(root)) Directory.Delete(root, true); }
    }

    [TestMethod]
    public void Write_failure_is_visible_without_returning_an_unwritten_path()
    {
        var root = Path.Combine(Path.GetTempPath(), "gpautolive-douyin-log-tests", Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try
        {
            var blocker = Path.Combine(root, "blocked");
            File.WriteAllText(blocker, "keep");
            var log = new WindowsDouyinDiagnosticLog(blocker);
            log.BeginRun();
            Assert.AreEqual(WindowsDouyinDiagnosticLogState.WriteFailed, log.Snapshot.State);
            Assert.IsNull(log.Snapshot.Path);
            Assert.AreEqual("keep", File.ReadAllText(blocker));
        }
        finally { Directory.Delete(root, true); }
    }

    [TestMethod]
    public void Arbitrary_diagnostic_values_are_never_written_and_volume_is_bounded()
    {
        var root = Path.Combine(Path.GetTempPath(), "gpautolive-douyin-log-tests", Guid.NewGuid().ToString("N"));
        try
        {
            var log = new WindowsDouyinDiagnosticLog(root);
            log.BeginRun();
            log.RecordDiagnostic(new("qr_fetch", "raw-secret-url-cookie-token"));
            for (var i = 0; i < 1000; i++) log.RecordDiagnostic(new("qr_poll", "network_error", "timeout"));
            var lines = File.ReadAllLines(log.Snapshot.Path!);
            Assert.AreEqual(66, lines.Length); // 启动 + 64 条受限诊断 + 单条达到上限。
            Assert.IsFalse(string.Join('\n', lines).Contains("raw-secret", StringComparison.Ordinal));
            Assert.IsTrue(lines[1].Contains("diagnostic_rejected", StringComparison.Ordinal));
            Assert.IsTrue(lines[^1].Contains("diagnostic_limit", StringComparison.Ordinal));
        }
        finally { if (Directory.Exists(root)) Directory.Delete(root, true); }
    }

    [TestMethod]
    public void Existing_log_failure_keeps_its_path_and_next_diagnostic_can_recover()
    {
        var root = Path.Combine(Path.GetTempPath(), "gpautolive-douyin-log-tests", Guid.NewGuid().ToString("N"));
        try
        {
            var log = new WindowsDouyinDiagnosticLog(root);
            log.BeginRun();
            var path = log.Snapshot.Path!;
            using (var held = new FileStream(path, FileMode.Open, FileAccess.ReadWrite, FileShare.None))
            {
                log.RecordDiagnostic(new("qr_fetch", "network_error", "timeout"));
                Assert.AreEqual(WindowsDouyinDiagnosticLogState.WriteFailed, log.Snapshot.State);
                Assert.AreEqual(path, log.Snapshot.Path);
            }
            log.RecordDiagnostic(new("qr_fetch", "qr_issued"));
            Assert.AreEqual(WindowsDouyinDiagnosticLogState.Ready, log.Snapshot.State);
            Assert.AreEqual(2, File.ReadAllLines(path).Length);
        }
        finally { if (Directory.Exists(root)) Directory.Delete(root, true); }
    }
}
