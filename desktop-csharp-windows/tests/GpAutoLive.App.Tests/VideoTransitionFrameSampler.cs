using System.Runtime.InteropServices;
using GpAutoLive.Windows;
using Vortice.Direct3D11;
using Windows.Graphics.Capture;

namespace GpAutoLive.App.Tests;

// Test-only synchronous readback: inspect every delivered WGC frame, without the production latest-frame converter.
internal sealed class VideoTransitionFrameSampler : IDisposable
{
    private ID3D11Texture2D? _staging;
    private readonly object _gate = new();
    private readonly List<string> _colors = [];
    private int _blackFrames;
    private string? _failure;
    private bool _armed;
    private long _previousTimestamp;
    private long _maximumGap100Ns;
    public (int Samples, int BlackFrames, string? Failure, string[] Colors, long MaximumGap100Ns) Snapshot
    {
        get { lock (_gate) { return (_colors.Count, _blackFrames, _failure, _colors.ToArray(), _maximumGap100Ns); } }
    }
    public void Arm() { lock (_gate) { _armed = true; _colors.Clear(); _blackFrames = 0; } }
    public void Accept(Direct3D11CaptureFrame frame, WindowsGraphicsCaptureD3D11Context context)
    {
        try
        {
            if (frame.Surface is not WinRT.IWinRTObject surface)
                throw new InvalidOperationException("WGC surface has no native interface.");
            var access = surface.NativeObject.AsInterface<IDirect3DDxgiInterfaceAccess>();
            try
            {
                var iid = new Guid("6f15aaf2-d208-4e89-9ab4-489535d34f9c");
                Marshal.ThrowExceptionForHR(access.GetInterface(in iid, out var pointer));
                using var source = new ID3D11Texture2D(pointer);
                var description = source.Description;
                if (_staging is null || _staging.Description.Width != description.Width || _staging.Description.Height != description.Height)
                {
                    _staging?.Dispose();
                    description.BindFlags = BindFlags.None;
                    description.Usage = ResourceUsage.Staging;
                    description.CPUAccessFlags = CpuAccessFlags.Read;
                    description.MiscFlags = ResourceOptionFlags.None;
                    _staging = context.Device.CreateTexture2D(in description);
                }
                context.ImmediateContext.CopyResource(_staging, source);
                var result = context.ImmediateContext.Map(_staging, 0, MapMode.Read, Vortice.Direct3D11.MapFlags.None, out var mapped);
                result.CheckError();
                string color;
                try
                {
                    // A central 32x32 ROI excludes title bars and aspect-ratio padding.
                    long red = 0, green = 0, blue = 0;
                    var row = new byte[32 * 4];
                    for (var y = 0; y < 32; y++)
                    {
                        var offset = checked((int)((description.Height / 2 - 16 + y) * mapped.RowPitch + (description.Width / 2 - 16) * 4));
                        Marshal.Copy(IntPtr.Add(mapped.DataPointer, offset), row, 0, row.Length);
                        for (var x = 0; x < 32; x++) { blue += row[x * 4]; green += row[x * 4 + 1]; red += row[x * 4 + 2]; }
                    }
                    color = Math.Max(red, Math.Max(green, blue)) < 24 * 1024 ? "black" : red > green * 2 && red > blue * 2 ? "red" : green > red * 2 && green > blue * 2 ? "green" : "other";
                }
                finally { context.ImmediateContext.Unmap(_staging, 0); }
                lock (_gate)
                {
                    if (_armed)
                    {
                        if (_colors.Count >= 20_000) throw new InvalidOperationException("Fixture sample limit exceeded.");
                        var timestamp = frame.SystemRelativeTime.Ticks;
                        if (_previousTimestamp != 0) _maximumGap100Ns = Math.Max(_maximumGap100Ns, timestamp - _previousTimestamp);
                        _previousTimestamp = timestamp;
                        _colors.Add(color); if (color == "black") _blackFrames++;
                    }
                    else { _colors.Clear(); _colors.Add(color); }
                }
            }
            finally { if (Marshal.IsComObject(access)) Marshal.ReleaseComObject(access); }
        }
        catch (Exception ex) { lock (_gate) { _failure = ex.GetType().Name + ": " + ex.Message; } }
    }
    public void Dispose() => _staging?.Dispose();
    [ComImport, Guid("a9b3d012-3df2-4ee3-b8d1-8695f457d3c1"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IDirect3DDxgiInterfaceAccess
    {
        [PreserveSig] int GetInterface(in Guid iid, out IntPtr pointer);
    }
}

