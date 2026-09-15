using System.Buffers.Binary;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraSidecarProtocolTests
{
    [TestMethod]
    [DataRow(1920U, 1080U)]
    [DataRow(1080U, 1920U)]
    [DataRow(4096U, 2160U)]
    [DataRow(640U, 481U)]
    public void V2_round_trip_preserves_adaptive_dimensions_and_v1_rejects_them(uint width, uint height)
    {
        var payload = new byte[checked((int)(width * height * 2))];
        var encoded = new byte[WindowsVirtualCameraSidecarProtocol.FrameHeaderBytes + payload.Length];
        var frame = new WindowsVirtualCameraSidecarProtocol.SidecarFrame(1, 1, 0, payload, width, height);
        Assert.IsTrue(WindowsVirtualCameraSidecarProtocol.TryEncode(frame, encoded, out var written, out var error), error?.Message);
        Assert.AreEqual(encoded.Length, written);
        Assert.AreEqual((ushort)2, BinaryPrimitives.ReadUInt16LittleEndian(encoded.AsSpan(8)));
        Assert.IsTrue(WindowsVirtualCameraSidecarProtocol.TryDecode(encoded, out var decoded, out var consumed, out error), error?.Message);
        Assert.AreEqual(width, decoded!.Width);
        Assert.AreEqual(height, decoded.Height);
        Assert.AreEqual(written, consumed);
        BinaryPrimitives.WriteUInt16LittleEndian(encoded.AsSpan(8), 1);
        Assert.IsFalse(WindowsVirtualCameraSidecarProtocol.TryDecode(encoded, out _, out _, out _));
    }

    [TestMethod]
    public void Decoder_rejects_oversized_or_odd_width_headers_before_payload_allocation()
    {
        foreach (var (width, height) in new[] { (641U, 480U), (4096U, 2161U), (2U, 4097U), (uint.MaxValue, uint.MaxValue) })
        {
            var encoded = ValidEncodedFrame();
            BinaryPrimitives.WriteUInt32LittleEndian(encoded.AsSpan(36), width);
            BinaryPrimitives.WriteUInt32LittleEndian(encoded.AsSpan(40), height);
            Assert.IsFalse(WindowsVirtualCameraSidecarProtocol.TryDecode(encoded, out _, out _, out var error));
            Assert.AreEqual("sidecar_frame_invalid", error!.Code);
        }
        var legacy = ValidEncodedFrame();
        BinaryPrimitives.WriteUInt16LittleEndian(legacy.AsSpan(8), 1);
        Assert.IsTrue(WindowsVirtualCameraSidecarProtocol.TryDecode(legacy, out _, out _, out _));
    }

    [TestMethod]
    public void Pipe_name_is_derived_from_fixed_nonzero_16_byte_token()
    {
        Assert.IsTrue(
            WindowsVirtualCameraSidecarProtocol.TryCreatePipeName(
                Enumerable.Repeat((byte)0xab, 16).ToArray(),
                out var pipeName,
                out var error),
            error?.Message);
        Assert.AreEqual(
            @"\\.\pipe\GpAutoLive-AkVirtualCamera-abababababababababababababababab",
            pipeName);
        Assert.IsTrue(WindowsVirtualCameraSidecarProtocol.TryValidatePipeName(pipeName, out error), error?.Message);
        Assert.IsFalse(WindowsVirtualCameraSidecarProtocol.TryValidatePipeName(@"\\.\pipe\other", out _));
        Assert.IsFalse(
            WindowsVirtualCameraSidecarProtocol.TryValidatePipeName(
                WindowsVirtualCameraSidecarProtocol.PipePrefix + new string('0', 32),
                out _));
    }

    [TestMethod]
    public void Encode_and_decode_round_trip_consumes_only_one_stream_frame()
    {
        var frame = new WindowsVirtualCameraSidecarProtocol.SidecarFrame(
            3,
            7,
            1234,
            Enumerable.Repeat((byte)16, (1280 * 720 * 2)).ToArray());
        var encoded = new byte[(WindowsVirtualCameraSidecarProtocol.FrameHeaderBytes + 1280 * 720 * 2) + 4];

        Assert.IsTrue(
            WindowsVirtualCameraSidecarProtocol.TryEncode(
                frame,
                encoded,
                out var written,
                out var encodeError),
            encodeError?.Message);
        encoded[written] = (byte)'n';
        encoded[written + 1] = (byte)'e';
        encoded[written + 2] = (byte)'x';
        encoded[written + 3] = (byte)'t';

        Assert.IsTrue(
            WindowsVirtualCameraSidecarProtocol.TryDecode(
                encoded,
                out var decoded,
                out var consumed,
                out var decodeError),
            decodeError?.Message);
        Assert.AreEqual((WindowsVirtualCameraSidecarProtocol.FrameHeaderBytes + 1280 * 720 * 2), consumed);
        Assert.AreEqual(frame.Generation, decoded!.Generation);
        Assert.AreEqual(frame.Sequence, decoded.Sequence);
        Assert.AreEqual(frame.Timestamp100Ns, decoded.Timestamp100Ns);
        CollectionAssert.AreEqual(frame.Payload, decoded.Payload);
    }

    [TestMethod]
    public void Truncated_input_is_rejected_before_payload_copy()
    {
        var input = new byte[WindowsVirtualCameraSidecarProtocol.FrameHeaderBytes];
        WindowsVirtualCameraSidecarProtocol.FrameMagic.CopyTo(input);
        BinaryPrimitives.WriteUInt16LittleEndian(input.AsSpan(8), WindowsVirtualCameraSidecarProtocol.ProtocolVersion);
        BinaryPrimitives.WriteUInt16LittleEndian(input.AsSpan(10), WindowsVirtualCameraSidecarProtocol.FrameHeaderBytes);
        BinaryPrimitives.WriteUInt64LittleEndian(input.AsSpan(12), 1);
        BinaryPrimitives.WriteUInt64LittleEndian(input.AsSpan(20), 1);
        BinaryPrimitives.WriteUInt32LittleEndian(input.AsSpan(36), WindowsVirtualCameraSidecarProtocol.OutputWidth);
        BinaryPrimitives.WriteUInt32LittleEndian(input.AsSpan(40), WindowsVirtualCameraSidecarProtocol.OutputHeight);
        BinaryPrimitives.WriteUInt32LittleEndian(input.AsSpan(44), (1280 * 720 * 2));

        Assert.IsFalse(WindowsVirtualCameraSidecarProtocol.TryDecode(input, out _, out _, out var error));
        Assert.AreEqual("sidecar_frame_truncated", error!.Code);
    }

    [TestMethod]
    public void Decode_rejects_reserved_field_and_invalid_identity()
    {
        var frame = ValidEncodedFrame();
        BinaryPrimitives.WriteUInt32LittleEndian(frame.AsSpan(48), 1);
        Assert.IsFalse(WindowsVirtualCameraSidecarProtocol.TryDecode(frame, out _, out _, out var reservedError));
        Assert.AreEqual("sidecar_frame_invalid", reservedError!.Code);

        frame = ValidEncodedFrame();
        BinaryPrimitives.WriteUInt64LittleEndian(frame.AsSpan(12), 0);
        Assert.IsFalse(WindowsVirtualCameraSidecarProtocol.TryDecode(frame, out _, out _, out var identityError));
        StringAssert.Contains(identityError!.Message, "不能为 0");
    }

    [TestMethod]
    public void Encode_rejects_small_destination_without_writing()
    {
        var destination = Enumerable.Repeat((byte)0xcd, (WindowsVirtualCameraSidecarProtocol.FrameHeaderBytes + 1280 * 720 * 2) - 1).ToArray();
        var before = destination[0];
        Assert.IsFalse(
            WindowsVirtualCameraSidecarProtocol.TryEncode(
                ValidFrame(),
                destination,
                out var written,
                out var error));
        Assert.AreEqual(0, written);
        Assert.AreEqual("sidecar_frame_buffer_too_small", error!.Code);
        Assert.AreEqual(before, destination[0]);
    }

    private static WindowsVirtualCameraSidecarProtocol.SidecarFrame ValidFrame() =>
        new(
            1,
            1,
            0,
            new byte[(1280 * 720 * 2)]);

    private static byte[] ValidEncodedFrame()
    {
        var encoded = new byte[(WindowsVirtualCameraSidecarProtocol.FrameHeaderBytes + 1280 * 720 * 2)];
        Assert.IsTrue(
            WindowsVirtualCameraSidecarProtocol.TryEncode(
                ValidFrame(),
                encoded,
                out _,
                out var error),
            error?.Message);
        return encoded;
    }
}
