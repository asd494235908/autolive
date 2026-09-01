//! 在 GPU 上把 BGRA 输出面打包为 YUY2 字节布局。
//!
//! 部分驱动只把 YUY2 报为 Video Processor 输入格式，不能直接创建 YUY2
//! 输出面。此模块用 Video Processor 先生成 BGRA，再用 D3D11 像素着色器
//! 将相邻两个像素打包成 `Y0 U Y1 V`，最后只回读一张 RGBA staging 纹理。
//! 着色器输出的四个通道就是 sidecar 所需的四个连续字节，因此 CPU 不做
//! 色彩转换，也不引入第二个播放器或逐帧编码。

use std::fmt;

use windows::core::{Error as WindowsError, PCSTR};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_SRV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Direct3D11::D3D11_SHADER_RESOURCE_VIEW_DESC_0 as ShaderResourceViewDescUnion;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader, ID3D11RenderTargetView,
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_FILTER_MIN_MAG_MIP_POINT,
    D3D11_SAMPLER_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_TEX2D_SRV, D3D11_TEXTURE2D_DESC,
    D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};

const VERTEX_SHADER: &[u8] = br#"
struct Output {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

Output main(uint vertex_id : SV_VertexID) {
    float2 positions[3] = {
        float2(-1.0, -1.0),
        float2(-1.0,  3.0),
        float2( 3.0, -1.0)
    };
    float2 coordinates[3] = {
        float2(0.0, 1.0),
        float2(0.0, -1.0),
        // The render target is half the source width because one packed
        // texel stores two source pixels.  This is a full-screen triangle,
        // so the overscan vertex must use 2.0 to make the target boundary
        // interpolate across the complete normalized source range 0..1.
        float2(2.0, 1.0)
    };
    Output output;
    output.position = float4(positions[vertex_id], 0.0, 1.0);
    output.uv = coordinates[vertex_id];
    return output;
}
"#;

pub(super) const PACK_SLOTS: usize = 3;
const FIXED_SOURCE_WIDTH: u32 = 1280;
const FIXED_SOURCE_HEIGHT: u32 = 720;

const PIXEL_SHADER: &[u8] = br#"
Texture2D source_texture : register(t0);
SamplerState source_sampler : register(s0);

float3 rgb_to_yuv(float3 rgb) {
    return float3(
        dot(rgb, float3(0.257, 0.504, 0.098)) + 0.0625,
        dot(rgb, float3(-0.148, -0.291, 0.439)) + 0.5,
        dot(rgb, float3(0.439, -0.368, -0.071)) + 0.5
    );
}

float4 main(float4 position : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    // Output width is half of the source width. Sample two adjacent source pixels.
    float2 source_pixel = float2(1.0 / 1280.0, 1.0 / 720.0);
    float3 left = rgb_to_yuv(source_texture.Sample(source_sampler, uv - float2(source_pixel.x * 0.5, 0.0)));
    float3 right = rgb_to_yuv(source_texture.Sample(source_sampler, uv + float2(source_pixel.x * 0.5, 0.0)));
    return float4(left.x, (left.y + right.y) * 0.5, right.x, (left.z + right.z) * 0.5);
}
"#;

#[derive(Debug)]
pub(super) enum GpuPackError {
    Windows(String),
    InvalidConfiguration(&'static str),
}

impl fmt::Display for GpuPackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Windows(message) => write!(formatter, "GPU YUY2 打包失败：{message}"),
            Self::InvalidConfiguration(message) => {
                write!(formatter, "GPU YUY2 打包配置无效：{message}")
            }
        }
    }
}

impl std::error::Error for GpuPackError {}

pub(super) struct GpuYuy2Pack {
    source_srv: ID3D11ShaderResourceView,
    packed_texture: ID3D11Texture2D,
    packed_rtv: ID3D11RenderTargetView,
    packed_staging: [ID3D11Texture2D; PACK_SLOTS],
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    output_width: u32,
    output_height: u32,
}

impl GpuYuy2Pack {
    pub(super) fn new(
        device: &ID3D11Device,
        source_texture: &ID3D11Texture2D,
        output_width: u32,
        output_height: u32,
    ) -> Result<Self, GpuPackError> {
        if output_width == 0 || output_height == 0 || !output_width.is_multiple_of(2) {
            return Err(GpuPackError::InvalidConfiguration(
                "YUY2 输出宽度必须是正偶数，尺寸不能为空",
            ));
        }
        if output_width != FIXED_SOURCE_WIDTH || output_height != FIXED_SOURCE_HEIGHT {
            return Err(GpuPackError::InvalidConfiguration(
                "GPU YUY2 打包首版只允许 1280×720 输出",
            ));
        }
        let source_srv = create_source_srv(device, source_texture)?;
        let packed_texture = create_texture(
            device,
            output_width / 2,
            output_height,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            D3D11_USAGE_DEFAULT,
            D3D11_BIND_RENDER_TARGET.0 as u32,
            0,
        )?;
        let mut packed_rtv = None;
        unsafe { device.CreateRenderTargetView(&packed_texture, None, Some(&mut packed_rtv)) }
            .map_err(|error| windows_error("创建 YUY2 打包渲染目标", error))?;
        let packed_rtv =
            packed_rtv.ok_or_else(|| GpuPackError::Windows("YUY2 打包渲染目标为空".to_owned()))?;
        let packed_staging = [
            create_texture(
                device,
                output_width / 2,
                output_height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                D3D11_USAGE_STAGING,
                0,
                D3D11_CPU_ACCESS_READ.0 as u32,
            )?,
            create_texture(
                device,
                output_width / 2,
                output_height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                D3D11_USAGE_STAGING,
                0,
                D3D11_CPU_ACCESS_READ.0 as u32,
            )?,
            create_texture(
                device,
                output_width / 2,
                output_height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                D3D11_USAGE_STAGING,
                0,
                D3D11_CPU_ACCESS_READ.0 as u32,
            )?,
        ];
        let vertex_shader = create_vertex_shader(device, VERTEX_SHADER)?;
        let pixel_shader = create_pixel_shader(device, PIXEL_SHADER)?;
        let sampler_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MinLOD: 0.0,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut sampler = None;
        unsafe { device.CreateSamplerState(&sampler_desc, Some(&mut sampler)) }
            .map_err(|error| windows_error("创建 YUY2 打包采样器", error))?;
        let sampler =
            sampler.ok_or_else(|| GpuPackError::Windows("YUY2 打包采样器为空".to_owned()))?;
        Ok(Self {
            source_srv,
            packed_texture,
            packed_rtv,
            packed_staging,
            vertex_shader,
            pixel_shader,
            sampler,
            output_width,
            output_height,
        })
    }

    pub(super) fn render(
        &self,
        context: &ID3D11DeviceContext,
        slot: usize,
    ) -> Result<(), GpuPackError> {
        let staging = self
            .packed_staging
            .get(slot)
            .ok_or(GpuPackError::InvalidConfiguration("GPU 打包槽位超出范围"))?;
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: (self.output_width / 2) as f32,
            Height: self.output_height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let render_targets = [Some(self.packed_rtv.clone())];
        let shader_resources = [Some(self.source_srv.clone())];
        let samplers = [Some(self.sampler.clone())];
        unsafe {
            context.OMSetRenderTargets(
                Some(&render_targets),
                None::<&windows::Win32::Graphics::Direct3D11::ID3D11DepthStencilView>,
            );
            context.RSSetViewports(Some(&[viewport]));
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.VSSetShader(Some(&self.vertex_shader), None);
            context.PSSetShader(Some(&self.pixel_shader), None);
            context.PSSetShaderResources(0, Some(&shader_resources));
            context.PSSetSamplers(0, Some(&samplers));
            context.Draw(3, 0);
            context.PSSetShaderResources(0, Some(&[None]));
            context.OMSetRenderTargets(
                Some(&[None]),
                None::<&windows::Win32::Graphics::Direct3D11::ID3D11DepthStencilView>,
            );
            context.CopyResource(staging, &self.packed_texture);
            context.Flush();
        }
        Ok(())
    }

    pub(super) fn staging_texture(&self, slot: usize) -> Option<&ID3D11Texture2D> {
        self.packed_staging.get(slot)
    }
}

fn create_source_srv(
    device: &ID3D11Device,
    source_texture: &ID3D11Texture2D,
) -> Result<ID3D11ShaderResourceView, GpuPackError> {
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        ViewDimension: D3D_SRV_DIMENSION_TEXTURE2D,
        Anonymous: ShaderResourceViewDescUnion {
            Texture2D: D3D11_TEX2D_SRV {
                MostDetailedMip: 0,
                MipLevels: 1,
            },
        },
    };
    let mut view = None;
    unsafe { device.CreateShaderResourceView(source_texture, Some(&desc), Some(&mut view)) }
        .map_err(|error| windows_error("创建 BGRA 着色器资源视图", error))?;
    view.ok_or_else(|| GpuPackError::Windows("BGRA 着色器资源视图为空".to_owned()))
}

fn create_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
    usage: windows::Win32::Graphics::Direct3D11::D3D11_USAGE,
    bind_flags: u32,
    cpu_access_flags: u32,
) -> Result<ID3D11Texture2D, GpuPackError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: usage,
        BindFlags: bind_flags,
        CPUAccessFlags: cpu_access_flags,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
        .map_err(|error| windows_error("创建 GPU YUY2 打包纹理", error))?;
    texture.ok_or_else(|| GpuPackError::Windows("GPU YUY2 打包纹理为空".to_owned()))
}

fn compile_shader(source: &[u8], target: &[u8]) -> Result<Vec<u8>, GpuPackError> {
    let entry = b"main\0";
    let source_name = b"autolive-gpu-yuy2-pack.hlsl\0";
    let mut blob = None;
    unsafe {
        D3DCompile(
            source.as_ptr() as *const core::ffi::c_void,
            source.len(),
            PCSTR(source_name.as_ptr()),
            None,
            None::<&windows::Win32::Graphics::Direct3D::ID3DInclude>,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            0,
            0,
            &mut blob,
            None,
        )
    }
    .map_err(|error| windows_error("编译 GPU YUY2 着色器", error))?;
    let blob = blob.ok_or_else(|| GpuPackError::Windows("GPU YUY2 着色器字节码为空".to_owned()))?;
    let pointer = unsafe { blob.GetBufferPointer() } as *const u8;
    let length = unsafe { blob.GetBufferSize() };
    if pointer.is_null() || length == 0 {
        return Err(GpuPackError::Windows(
            "GPU YUY2 着色器字节码无效".to_owned(),
        ));
    }
    Ok(unsafe { std::slice::from_raw_parts(pointer, length) }.to_vec())
}

fn create_vertex_shader(
    device: &ID3D11Device,
    source: &[u8],
) -> Result<ID3D11VertexShader, GpuPackError> {
    let bytecode = compile_shader(source, b"vs_5_0\0")?;
    let mut shader = None;
    unsafe { device.CreateVertexShader(&bytecode, None, Some(&mut shader)) }
        .map_err(|error| windows_error("创建 GPU YUY2 顶点着色器", error))?;
    shader.ok_or_else(|| GpuPackError::Windows("GPU YUY2 顶点着色器为空".to_owned()))
}

fn create_pixel_shader(
    device: &ID3D11Device,
    source: &[u8],
) -> Result<ID3D11PixelShader, GpuPackError> {
    let bytecode = compile_shader(source, b"ps_5_0\0")?;
    let mut shader = None;
    unsafe { device.CreatePixelShader(&bytecode, None, Some(&mut shader)) }
        .map_err(|error| windows_error("创建 GPU YUY2 像素着色器", error))?;
    shader.ok_or_else(|| GpuPackError::Windows("GPU YUY2 像素着色器为空".to_owned()))
}

fn windows_error(context: &str, error: WindowsError) -> GpuPackError {
    GpuPackError::Windows(format!("{context}：{error}"))
}

#[cfg(test)]
mod tests {
    use super::{FIXED_SOURCE_HEIGHT, FIXED_SOURCE_WIDTH, PIXEL_SHADER, VERTEX_SHADER};

    #[test]
    fn packed_target_maps_normalized_uvs_across_the_source_once() {
        let vertex_shader = String::from_utf8_lossy(VERTEX_SHADER);
        assert!(vertex_shader.contains("float2(2.0, 1.0)"));

        let pixel_shader = String::from_utf8_lossy(PIXEL_SHADER);
        assert!(pixel_shader.contains("1.0 / 1280.0"));
        assert!(pixel_shader.contains("1.0 / 720.0"));
        assert_eq!(FIXED_SOURCE_WIDTH, 1280);
        assert_eq!(FIXED_SOURCE_HEIGHT, 720);
    }
}
