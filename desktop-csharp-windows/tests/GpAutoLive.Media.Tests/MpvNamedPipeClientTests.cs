using System.IO.Pipes;
using System.Text.Json;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class MpvNamedPipeClientTests
{
    [TestMethod]
    public void EndpointAcceptsOnlyCanonicalWindowsPipeNames()
    {
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(
            @"\\.\pipe\autolive-test_01",
            out var endpoint,
            out var error));
        Assert.IsNull(error);
        Assert.IsNotNull(endpoint);
        Assert.AreEqual("autolive-test_01", endpoint!.PipeName);
        Assert.AreEqual(@"\\.\pipe\autolive-test_01", endpoint.PipePath);

        foreach (var invalid in new[]
        {
            "",
            "relative-pipe",
            @"C:\temp\mpv.sock",
            @"\\.\pipe\with\child",
            @"\\.\pipe\unsafe|name",
            @"\\.\pipe\bad name",
            @"\\.\pipe\  leading",
        })
        {
            Assert.IsFalse(MpvIpcPipeEndpoint.TryCreate(invalid, out _, out var invalidError), invalid);
            Assert.AreEqual(MpvIpcFailureCode.InvalidPipeName, invalidError?.Code, invalid);
        }
    }

    [TestMethod]
    public async Task ConnectedClientPairsResponseWithRequestIdAndSkipsEvents()
    {
        var endpoint = CreateEndpoint();
        await using var server = CreateServer(endpoint.PipeName);
        await using var client = new MpvNamedPipeClient(endpoint, FastOptions());

        var serverTask = Task.Run(async () =>
        {
            await server.WaitForConnectionAsync();
            using var reader = new StreamReader(server, leaveOpen: true);
            using var writer = new StreamWriter(server, leaveOpen: true) { AutoFlush = true };
            var requestLine = await reader.ReadLineAsync();
            Assert.IsNotNull(requestLine);
            using var request = JsonDocument.Parse(requestLine!);
            Assert.AreEqual(42UL, request.RootElement.GetProperty("request_id").GetUInt64());
            await writer.WriteLineAsync("{\"event\":\"property-change\",\"id\":1,\"name\":\"time-pos\",\"data\":1.0}");
            await writer.WriteLineAsync("{\"error\":\"success\",\"request_id\":42,\"data\":2.5}");
        });

        var connected = await client.ConnectAsync();
        Assert.IsTrue(connected.IsSuccess);
        Assert.AreEqual(MpvIpcPipeState.Connected, client.State);

        var result = await client.ExecuteAsync(
            42,
            MpvIpcCommand.GetProperty(MpvIpcProperty.PlaybackTime));

        Assert.IsTrue(result.IsSuccess);
        Assert.AreEqual(2.5, result.Frame?.Data?.GetDouble());
        await serverTask;
    }

    [TestMethod]
    public async Task ResponseTimeoutClosesConnectionWithoutLeakingPipePath()
    {
        var endpoint = CreateEndpoint();
        await using var server = CreateServer(endpoint.PipeName);
        await using var client = new MpvNamedPipeClient(endpoint, FastOptions(responseTimeout: TimeSpan.FromMilliseconds(120)));
        var serverTask = Task.Run(async () =>
        {
            await server.WaitForConnectionAsync();
            using var reader = new StreamReader(server, leaveOpen: true);
            _ = await reader.ReadLineAsync();
            await Task.Delay(TimeSpan.FromMilliseconds(400));
        });

        Assert.IsTrue((await client.ConnectAsync()).IsSuccess);
        var result = await client.ExecuteAsync(
            7,
            MpvIpcCommand.GetProperty(MpvIpcProperty.PlaybackTime));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MpvIpcFailureCode.IpcTimeout, result.Error?.Code);
        Assert.IsFalse(result.Error?.Message.Contains(endpoint.PipePath, StringComparison.OrdinalIgnoreCase) == true);
        Assert.AreEqual(MpvIpcPipeState.Disconnected, client.State);
        await serverTask;
    }

    [TestMethod]
    public async Task CallerCancellationClosesConnectionWithStableCancellationCode()
    {
        var endpoint = CreateEndpoint();
        await using var server = CreateServer(endpoint.PipeName);
        await using var client = new MpvNamedPipeClient(endpoint, FastOptions(responseTimeout: TimeSpan.FromSeconds(5)));
        var serverTask = Task.Run(async () =>
        {
            await server.WaitForConnectionAsync();
            using var reader = new StreamReader(server, leaveOpen: true);
            _ = await reader.ReadLineAsync();
            await Task.Delay(TimeSpan.FromSeconds(1));
        });

        Assert.IsTrue((await client.ConnectAsync()).IsSuccess);
        using var cancellation = new CancellationTokenSource(TimeSpan.FromMilliseconds(100));
        var result = await client.ExecuteAsync(
            8,
            MpvIpcCommand.GetProperty(MpvIpcProperty.PlaybackTime),
            cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MpvIpcFailureCode.IpcCancelled, result.Error?.Code);
        Assert.AreEqual(MpvIpcPipeState.Disconnected, client.State);
        await serverTask;
    }

    [TestMethod]
    public async Task ResponseRequestIdMismatchIsRejectedAndDisconnects()
    {
        var endpoint = CreateEndpoint();
        await using var server = CreateServer(endpoint.PipeName);
        await using var client = new MpvNamedPipeClient(endpoint, FastOptions());
        var serverTask = Task.Run(async () =>
        {
            await server.WaitForConnectionAsync();
            using var reader = new StreamReader(server, leaveOpen: true);
            using var writer = new StreamWriter(server, leaveOpen: true) { AutoFlush = true };
            _ = await reader.ReadLineAsync();
            await writer.WriteLineAsync("{\"error\":\"success\",\"request_id\":99,\"data\":true}");
        });

        Assert.IsTrue((await client.ConnectAsync()).IsSuccess);
        var result = await client.ExecuteAsync(
            9,
            MpvIpcCommand.GetProperty(MpvIpcProperty.Paused));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MpvIpcFailureCode.ResponseRequestIdMismatch, result.Error?.Code);
        Assert.AreEqual(9UL, result.Error?.RequestId);
        Assert.AreEqual(MpvIpcPipeState.Faulted, client.State);
        await serverTask;
    }

    [TestMethod]
    public async Task UnknownResponseFieldIsRejectedWithoutEchoingRawFrame()
    {
        var endpoint = CreateEndpoint();
        await using var server = CreateServer(endpoint.PipeName);
        await using var client = new MpvNamedPipeClient(endpoint, FastOptions());
        var serverTask = Task.Run(async () =>
        {
            await server.WaitForConnectionAsync();
            using var reader = new StreamReader(server, leaveOpen: true);
            using var writer = new StreamWriter(server, leaveOpen: true) { AutoFlush = true };
            _ = await reader.ReadLineAsync();
            await writer.WriteLineAsync("{\"error\":\"success\",\"request_id\":10,\"secret_path\":\"C:\\\\private\\\\file.mp4\"}");
        });

        Assert.IsTrue((await client.ConnectAsync()).IsSuccess);
        var result = await client.ExecuteAsync(
            10,
            MpvIpcCommand.GetProperty(MpvIpcProperty.PlaybackTime));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MpvIpcFailureCode.UnknownField, result.Error?.Code);
        Assert.IsFalse(result.Error?.Message.Contains("private", StringComparison.OrdinalIgnoreCase) == true);
        await serverTask;
    }

    [TestMethod]
    public async Task DisconnectReturnsSanitizedStableError()
    {
        var endpoint = CreateEndpoint();
        await using var server = CreateServer(endpoint.PipeName);
        await using var client = new MpvNamedPipeClient(endpoint, FastOptions());
        var serverTask = Task.Run(async () =>
        {
            await server.WaitForConnectionAsync();
            using var reader = new StreamReader(server, leaveOpen: true);
            _ = await reader.ReadLineAsync();
            server.Disconnect();
        });

        Assert.IsTrue((await client.ConnectAsync()).IsSuccess);
        var result = await client.ExecuteAsync(
            11,
            MpvIpcCommand.GetProperty(MpvIpcProperty.PlaybackTime));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MpvIpcFailureCode.IpcDisconnected, result.Error?.Code);
        Assert.IsFalse(result.Error?.Message.Contains(endpoint.PipeName, StringComparison.OrdinalIgnoreCase) == true);
        await serverTask;
    }

    private static MpvIpcPipeEndpoint CreateEndpoint()
    {
        var path = $@"\\.\pipe\autolive-test-{Guid.NewGuid():N}";
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(path, out var endpoint, out var error));
        Assert.IsNull(error);
        Assert.IsNotNull(endpoint);
        return endpoint!;
    }

    private static NamedPipeServerStream CreateServer(string pipeName) => new(
        pipeName,
        PipeDirection.InOut,
        maxNumberOfServerInstances: 1,
        PipeTransmissionMode.Byte,
        PipeOptions.Asynchronous);

    private static MpvIpcPipeOptions FastOptions(TimeSpan? responseTimeout = null) => new()
    {
        ConnectTimeout = TimeSpan.FromSeconds(2),
        ResponseTimeout = responseTimeout ?? TimeSpan.FromSeconds(2),
        MaxFrameBytes = 4 * 1024,
    };
}
