//! Windows 原生虚拟摄像头准入探测。
//!
//! 这个 crate 提供平台和已安装 sidecar 的前置条件探测，以及受控的 Windows
//! Graphics Capture/D3D11 三槽 YUY2 原生捕获边界；它不启动 AkVirtualCamera
//! 进程，也不宣称签名、安装或下游兼容门禁已经通过。实际输出链必须在探测
//! 结果和独立的 GPU 转换/回读门禁全部通过后才能接入。

use std::path::PathBuf;

mod capture;
pub use capture::{
    CaptureConfig, CaptureError, CaptureGpuFacts, CapturedFrame, NativeCapturePump,
    NativeCaptureSession,
};
#[cfg(windows)]
pub(crate) mod gpu_yuy2_pack;
mod sidecar_protocol;
pub use sidecar_protocol::{
    pipe_name as virtual_camera_pipe_name, validate_pipe_name as validate_virtual_camera_pipe_name,
    ProtocolError as VirtualCameraProtocolError, SidecarFrame, FRAME_HEADER_BYTES, FRAME_MAGIC,
    MAX_PAYLOAD_BYTES, OUTPUT_HEIGHT, OUTPUT_WIDTH, PROTOCOL_VERSION,
};

pub const WINDOWS_GRAPHICS_CAPTURE_API: &str = "windows_graphics_capture";
pub const MIN_WINDOWS_BUILD: u32 = 18_362;
pub const AKVIRTUALCAMERA_TRANSPORT: &str = "akvcam_mmap_cpu";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarArchitecture {
    X86,
    X64,
    Unknown,
}

impl SidecarArchitecture {
    pub const fn label(self) -> &'static str {
        match self {
            Self::X86 => "x86",
            Self::X64 => "x64",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeProbeConfig {
    /// 应用唯一最终效果窗口的 HWND；不提供时探测必须保持未就绪。
    pub final_effect_window_id: Option<u64>,
    /// 本地 GPL sidecar 的可执行文件路径；不提供时输出链保持未就绪。
    pub sidecar_path: Option<PathBuf>,
    /// 当前进程要启动的 sidecar 架构。主桌面端默认使用 x64，32 位下游组件
    /// 由独立的 DirectShow 组件承接，不把主进程改成 x86。
    pub expected_sidecar_architecture: SidecarArchitecture,
}

impl Default for NativeProbeConfig {
    fn default() -> Self {
        Self {
            final_effect_window_id: None,
            sidecar_path: None,
            expected_sidecar_architecture: SidecarArchitecture::X64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsProbe {
    pub supported: bool,
    pub major: Option<u32>,
    pub minor: Option<u32>,
    pub build: Option<u32>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowProbe {
    pub requested_window_id: Option<u64>,
    pub valid: bool,
    pub visible: bool,
    pub client_width: u32,
    pub client_height: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WgcProbe {
    pub api: &'static str,
    pub runtime_supported: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterProbe {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub luid: String,
    pub feature_level: String,
    pub is_warp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3d11Probe {
    pub available: bool,
    pub adapter: Option<AdapterProbe>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarProbe {
    pub requested_path: Option<String>,
    pub present: bool,
    pub regular_file: bool,
    pub architecture: SidecarArchitecture,
    pub expected_architecture: SidecarArchitecture,
    /// 仅表示 PE 文件头和架构匹配；不等同于 Authenticode 签名验证。
    pub usable_for_development: bool,
    /// 签名必须由发布/安装门禁验证，原生探测不把“未检查”当成通过。
    pub authenticode_verified: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeProbeReport {
    pub os: OsProbe,
    pub window: WindowProbe,
    pub wgc: WgcProbe,
    pub d3d11: D3d11Probe,
    pub sidecar: SidecarProbe,
    /// WGC、HWND、硬件 D3D11 和开发期 sidecar 均满足时为 true。
    /// 这仍不是完整输出就绪：捕获模块的实机性能、设备注册和投递需要后续门禁。
    pub prerequisites_ready: bool,
    /// 当前恒为 false，直到独立发布门禁验证 Authenticode 和 GPL 材料。
    pub release_ready: bool,
    pub blockers: Vec<String>,
}

impl NativeProbeReport {
    pub fn can_start_capture(&self) -> bool {
        self.os.supported
            && self.window.valid
            && self.window.visible
            && self.window.client_width > 0
            && self.window.client_height > 0
            && self.window.error.is_none()
            && self.wgc.runtime_supported
            && self.d3d11.available
    }

    pub fn can_start_development_output(&self) -> bool {
        self.prerequisites_ready
    }

    pub fn is_release_ready(&self) -> bool {
        self.release_ready
    }
}

/// 探测 Windows 原生输出链的前置条件。
pub fn probe(config: &NativeProbeConfig) -> NativeProbeReport {
    platform::probe(config)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};
    use std::mem::size_of;

    use windows::core::Interface;
    use windows::Graphics::Capture::GraphicsCaptureSession;
    use windows::Win32::Foundation::{HWND, LUID, RECT};
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
    };
    use windows::Win32::System::SystemInformation::{GetVersionExW, OSVERSIONINFOW};
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsWindow, IsWindowVisible};

    pub(super) fn probe(config: &NativeProbeConfig) -> NativeProbeReport {
        let os = probe_os();
        let window = probe_window(config.final_effect_window_id);
        let wgc = probe_wgc(os.supported);
        let d3d11 = probe_d3d11();
        let sidecar = probe_sidecar(
            config.sidecar_path.as_deref(),
            config.expected_sidecar_architecture,
        );

        let mut blockers = Vec::new();
        if !os.supported {
            blockers.push("Windows 10 1903（Build 18362）或更高版本不可用".to_owned());
        }
        if !window.valid || !window.visible {
            blockers.push(
                window
                    .error
                    .clone()
                    .unwrap_or_else(|| "最终效果 HWND 不可捕获".to_owned()),
            );
        }
        if !wgc.runtime_supported {
            blockers.push(
                wgc.error
                    .clone()
                    .unwrap_or_else(|| "Windows Graphics Capture 不可用".to_owned()),
            );
        }
        if !d3d11.available {
            blockers.push(
                d3d11
                    .error
                    .clone()
                    .unwrap_or_else(|| "硬件 D3D11 不可用".to_owned()),
            );
        }
        if !sidecar.usable_for_development {
            blockers.push(
                sidecar
                    .error
                    .clone()
                    .unwrap_or_else(|| "AkVirtualCamera sidecar 不可用".to_owned()),
            );
        }

        // Authenticode、GPL 对应源码和安装/注册材料不在此处猜测。即使开发期前置
        // 条件齐全，发布门禁仍必须显式验证，因此 release_ready 固定为 false。
        blockers
            .push("GPU 转换/三槽 staging 仅完成本机短测，尚未完成发布矩阵与报告归档".to_owned());
        blockers.push("AkVirtualCamera Authenticode/GPL 发布材料尚未通过发布门禁".to_owned());

        let prerequisites_ready = os.supported
            && window.valid
            && window.visible
            && wgc.runtime_supported
            && d3d11.available
            && sidecar.usable_for_development;

        NativeProbeReport {
            os,
            window,
            wgc,
            d3d11,
            sidecar,
            prerequisites_ready,
            release_ready: false,
            blockers,
        }
    }

    fn probe_os() -> OsProbe {
        let mut info = OSVERSIONINFOW {
            dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        // SAFETY: info 是由 Rust 完整初始化的可写结构，Windows 只写入该出参。
        match unsafe { GetVersionExW(&mut info) } {
            Ok(()) => OsProbe {
                supported: info.dwMajorVersion >= 10 && info.dwBuildNumber >= MIN_WINDOWS_BUILD,
                major: Some(info.dwMajorVersion),
                minor: Some(info.dwMinorVersion),
                build: Some(info.dwBuildNumber),
                error: None,
            },
            Err(error) => OsProbe {
                supported: false,
                major: None,
                minor: None,
                build: None,
                error: Some(format!("读取 Windows 版本失败：{error}")),
            },
        }
    }

    fn probe_window(window_id: Option<u64>) -> WindowProbe {
        let mut result = WindowProbe {
            requested_window_id: window_id,
            valid: false,
            visible: false,
            client_width: 0,
            client_height: 0,
            error: None,
        };
        let Some(window_id) = window_id else {
            result.error = Some("未提供应用唯一最终效果 HWND".to_owned());
            return result;
        };
        let Ok(window_value) = usize::try_from(window_id) else {
            result.error = Some("最终效果 HWND 超出当前进程指针宽度".to_owned());
            return result;
        };
        if window_value == 0 {
            result.error = Some("最终效果 HWND 为 0".to_owned());
            return result;
        }
        let window = HWND(window_value as *mut c_void);
        // SAFETY: HWND 仅由调用方传入的不透明整数构造；IsWindow 不解引用调用方内存。
        if !unsafe { IsWindow(Some(window)).as_bool() } {
            result.error = Some("最终效果 HWND 不是有效窗口".to_owned());
            return result;
        }
        result.valid = true;
        // SAFETY: window 已由 IsWindow 验证；可见性查询不转移句柄所有权。
        result.visible = unsafe { IsWindowVisible(window).as_bool() };
        let mut rect = RECT::default();
        // SAFETY: rect 是有效可写出参，window 已验证有效。
        if let Err(error) = unsafe { GetClientRect(window, &mut rect) } {
            result.error = Some(format!("读取最终效果 HWND 客户区失败：{error}"));
            return result;
        }
        result.client_width = rect.right.saturating_sub(rect.left) as u32;
        result.client_height = rect.bottom.saturating_sub(rect.top) as u32;
        if !result.visible {
            result.error = Some("最终效果 HWND 当前不可见".to_owned());
        } else if result.client_width == 0 || result.client_height == 0 {
            result.error = Some("最终效果 HWND 客户区尺寸为 0".to_owned());
        }
        result
    }

    fn probe_wgc(os_supported: bool) -> WgcProbe {
        let mut result = WgcProbe {
            api: WINDOWS_GRAPHICS_CAPTURE_API,
            runtime_supported: false,
            error: None,
        };
        if !os_supported {
            result.error = Some("当前 Windows 版本不满足 WGC 最低版本".to_owned());
            return result;
        }
        // SAFETY: 当前探测调用在本线程建立 MTA；成功后对称调用 RoUninitialize。
        let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        if let Err(error) = initialized {
            result.error = Some(format!("初始化 WinRT MTA 失败：{error}"));
            return result;
        }
        let support = GraphicsCaptureSession::IsSupported();
        // SAFETY: 上面的 RoInitialize 成功，当前线程的对应初始化引用在此释放。
        unsafe { RoUninitialize() };
        match support {
            Ok(true) => result.runtime_supported = true,
            Ok(false) => result.error = Some("Windows Graphics Capture 报告为不支持".to_owned()),
            Err(error) => result.error = Some(format!("查询 WGC 支持状态失败：{error}")),
        }
        result
    }

    fn probe_d3d11() -> D3d11Probe {
        // SAFETY: CreateDXGIFactory1 只创建 COM 工厂对象，返回值由 windows crate 管理引用计数。
        let factory = match unsafe { CreateDXGIFactory1::<IDXGIFactory1>() } {
            Ok(factory) => factory,
            Err(error) => {
                return D3d11Probe {
                    available: false,
                    adapter: None,
                    error: Some(format!("创建 DXGI 工厂失败：{error}")),
                };
            }
        };

        let mut first_error = None;
        for index in 0..32 {
            // SAFETY: 工厂对象由本函数持有；EnumAdapters1 返回受 windows crate 管理的接口。
            let adapter = match unsafe { factory.EnumAdapters1(index) } {
                Ok(adapter) => adapter,
                Err(error) => {
                    if index == 0 {
                        first_error = Some(format!("枚举 D3D11 adapter 失败：{error}"));
                    }
                    break;
                }
            };
            // SAFETY: adapter 是枚举得到的有效 COM 接口，GetDesc1 只写入返回结构。
            let description = match unsafe { adapter.GetDesc1() } {
                Ok(description) => description,
                Err(error) => {
                    first_error
                        .get_or_insert_with(|| format!("读取 D3D11 adapter 描述失败：{error}"));
                    continue;
                }
            };
            if (description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            // D3D11CreateDevice 的参数契约要求 IDXGIAdapter 基接口；EnumAdapters1
            // 返回 IDXGIAdapter1，显式 QueryInterface 保持边界可见。
            let adapter_base = match adapter.cast::<IDXGIAdapter>() {
                Ok(adapter_base) => adapter_base,
                Err(error) => {
                    first_error
                        .get_or_insert_with(|| format!("转换 D3D11 adapter 接口失败：{error}"));
                    continue;
                }
            };
            let feature_levels = [D3D_FEATURE_LEVEL_11_0];
            let mut device = None;
            let mut context = None;
            let mut feature_level = D3D_FEATURE_LEVEL(0);
            // SAFETY: 输出 Option 接收槽由本函数独占；adapter、feature_levels 在同步调用期间有效。
            let created = unsafe {
                D3D11CreateDevice(
                    Some(&adapter_base),
                    D3D_DRIVER_TYPE_UNKNOWN,
                    Default::default(),
                    D3D11_CREATE_DEVICE_FLAG(0),
                    Some(&feature_levels),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    Some(&mut feature_level),
                    Some(&mut context),
                )
            };
            if let Err(error) = created {
                first_error.get_or_insert_with(|| format!("创建硬件 D3D11 设备失败：{error}"));
                continue;
            }
            if device.is_none() || context.is_none() || feature_level.0 < D3D_FEATURE_LEVEL_11_0.0 {
                first_error
                    .get_or_insert_with(|| "D3D11 返回的设备或 feature level 无效".to_owned());
                continue;
            }
            return D3d11Probe {
                available: true,
                adapter: Some(AdapterProbe {
                    name: utf16_string(&description.Description),
                    vendor_id: description.VendorId,
                    device_id: description.DeviceId,
                    luid: format_luid(description.AdapterLuid),
                    feature_level: format!("0x{:04x}", feature_level.0),
                    is_warp: false,
                }),
                error: None,
            };
        }
        D3d11Probe {
            available: false,
            adapter: None,
            error: first_error.or_else(|| Some("没有可用的硬件 D3D11 adapter".to_owned())),
        }
    }

    fn probe_sidecar(
        path: Option<&std::path::Path>,
        expected: SidecarArchitecture,
    ) -> SidecarProbe {
        let mut result = SidecarProbe {
            requested_path: path.map(|value| value.display().to_string()),
            present: false,
            regular_file: false,
            architecture: SidecarArchitecture::Unknown,
            expected_architecture: expected,
            usable_for_development: false,
            authenticode_verified: false,
            error: None,
        };
        let Some(path) = path else {
            result.error = Some("未提供 AkVirtualCamera sidecar 路径".to_owned());
            return result;
        };
        if expected == SidecarArchitecture::Unknown {
            result.error = Some("未指定 AkVirtualCamera sidecar 目标架构".to_owned());
            return result;
        }
        match std::fs::metadata(path) {
            Ok(metadata) => {
                result.present = true;
                result.regular_file = metadata.is_file();
                if !result.regular_file {
                    result.error = Some("AkVirtualCamera sidecar 路径不是普通文件".to_owned());
                    return result;
                }
            }
            Err(error) => {
                result.error = Some(format!("读取 AkVirtualCamera sidecar 失败：{error}"));
                return result;
            }
        }
        match read_pe_architecture(path) {
            Ok(architecture) => result.architecture = architecture,
            Err(error) => {
                result.error = Some(format!(
                    "AkVirtualCamera sidecar 不是可识别 PE 文件：{error}"
                ));
                return result;
            }
        }
        if result.architecture == SidecarArchitecture::Unknown {
            result.error = Some("AkVirtualCamera sidecar 使用了不支持的 PE 架构".to_owned());
            return result;
        }
        if result.architecture != expected {
            result.error = Some(format!(
                "AkVirtualCamera sidecar 架构不匹配：需要 {}，实际 {}",
                expected.label(),
                result.architecture.label()
            ));
            return result;
        }
        result.usable_for_development = true;
        result
    }

    fn read_pe_architecture(path: &std::path::Path) -> Result<SidecarArchitecture, String> {
        let mut file = File::open(path).map_err(|error| error.to_string())?;
        let mut dos_header = [0_u8; 64];
        file.read_exact(&mut dos_header)
            .map_err(|error| error.to_string())?;
        if &dos_header[0..2] != b"MZ" {
            return Err("DOS 标记缺失".to_owned());
        }
        let pe_offset = u32::from_le_bytes([
            dos_header[0x3c],
            dos_header[0x3d],
            dos_header[0x3e],
            dos_header[0x3f],
        ]) as u64;
        if pe_offset > 1_048_576 {
            return Err("PE 头偏移超出受控范围".to_owned());
        }
        file.seek(SeekFrom::Start(pe_offset))
            .map_err(|error| error.to_string())?;
        let mut pe_header = [0_u8; 6];
        file.read_exact(&mut pe_header)
            .map_err(|error| error.to_string())?;
        if &pe_header[0..4] != b"PE\0\0" {
            return Err("PE 标记缺失".to_owned());
        }
        let machine = u16::from_le_bytes([pe_header[4], pe_header[5]]);
        match machine {
            0x014c => Ok(SidecarArchitecture::X86),
            0x8664 => Ok(SidecarArchitecture::X64),
            _ => Ok(SidecarArchitecture::Unknown),
        }
    }

    fn utf16_string(value: &[u16]) -> String {
        let length = value
            .iter()
            .position(|item| *item == 0)
            .unwrap_or(value.len());
        String::from_utf16_lossy(&value[..length])
    }

    fn format_luid(value: LUID) -> String {
        format!("{:08x}:{:08x}", value.HighPart as u32, value.LowPart)
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub(super) fn probe(config: &NativeProbeConfig) -> NativeProbeReport {
        NativeProbeReport {
            os: OsProbe {
                supported: false,
                major: None,
                minor: None,
                build: None,
                error: Some("虚拟摄像头原生链仅支持 Windows 10/11".to_owned()),
            },
            window: WindowProbe {
                requested_window_id: config.final_effect_window_id,
                valid: false,
                visible: false,
                client_width: 0,
                client_height: 0,
                error: Some("非 Windows 平台没有 WGC HWND 探测实现".to_owned()),
            },
            wgc: WgcProbe {
                api: WINDOWS_GRAPHICS_CAPTURE_API,
                runtime_supported: false,
                error: Some("Windows Graphics Capture 仅支持 Windows".to_owned()),
            },
            d3d11: D3d11Probe {
                available: false,
                adapter: None,
                error: Some("D3D11 原生探测仅支持 Windows".to_owned()),
            },
            sidecar: SidecarProbe {
                requested_path: config
                    .sidecar_path
                    .as_ref()
                    .map(|value| value.display().to_string()),
                present: false,
                regular_file: false,
                architecture: SidecarArchitecture::Unknown,
                expected_architecture: config.expected_sidecar_architecture,
                usable_for_development: false,
                authenticode_verified: false,
                error: Some("AkVirtualCamera sidecar 仅支持 Windows PE".to_owned()),
            },
            prerequisites_ready: false,
            release_ready: false,
            blockers: vec!["虚拟摄像头原生链仅支持 Windows 10/11".to_owned()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_requires_real_window_and_sidecar() {
        let config = NativeProbeConfig::default();
        assert_eq!(
            config.expected_sidecar_architecture,
            SidecarArchitecture::X64
        );
        assert!(config.final_effect_window_id.is_none());
        assert!(config.sidecar_path.is_none());
    }

    #[test]
    fn unsupported_or_incomplete_probe_never_reports_ready() {
        let report = probe(&NativeProbeConfig::default());
        assert!(!report.prerequisites_ready);
        assert!(!report.release_ready);
        assert!(!report.can_start_development_output());
        assert!(!report.is_release_ready());
        assert!(!report.blockers.is_empty());
    }
}
