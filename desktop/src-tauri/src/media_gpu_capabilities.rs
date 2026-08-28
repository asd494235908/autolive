use serde::{Deserialize, Serialize};
use std::ffi::OsString;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FfmpegProbeKind {
    D3d11vaAmf,
    VulkanLibplacebo,
    Nvenc,
    Amf,
    Qsv,
    MediaFoundation,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegProbeCommand {
    pub kind: FfmpegProbeKind,
    pub args: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfmpegProbeResult {
    pub kind: FfmpegProbeKind,
    pub available: bool,
    pub reason: Option<String>,
}

impl FfmpegProbeResult {
    pub fn available(kind: FfmpegProbeKind) -> Self {
        Self {
            kind,
            available: true,
            reason: None,
        }
    }

    pub fn unavailable(kind: FfmpegProbeKind, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            kind,
            available: false,
            reason: Some(if reason.trim().is_empty() {
                "探测失败（无详细原因）".to_owned()
            } else {
                reason
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfmpegCapabilityState {
    pub probed: bool,
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FfmpegRenderBackend {
    FfmpegGpu,
    FfmpegCpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FfmpegGpuPath {
    D3d11vaAmf,
    VulkanLibplacebo,
    Nvenc,
    Amf,
    Qsv,
    MediaFoundation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfmpegGpuCapabilityReport {
    pub d3d11va: FfmpegCapabilityState,
    pub vulkan_libplacebo: FfmpegCapabilityState,
    pub amf: FfmpegCapabilityState,
    pub nvenc: FfmpegCapabilityState,
    pub qsv: FfmpegCapabilityState,
    pub media_foundation: FfmpegCapabilityState,
    pub cpu: FfmpegCapabilityState,
    /// 按 NVENC -> AMF -> QSV -> Media Foundation -> 软件选出的首个实测编码器。
    pub selected_encoder: Option<String>,
    pub selected_backend: FfmpegRenderBackend,
    pub selected_gpu_path: Option<FfmpegGpuPath>,
    pub fallback_reason: Option<String>,
}

pub fn ffmpeg_probe_commands() -> Vec<FfmpegProbeCommand> {
    let mut commands = vec![
        probe_command(
            FfmpegProbeKind::Nvenc,
            &[],
            None,
            &["-c:v", "h264_nvenc", "-preset", "p4", "-tune", "ll"],
        ),
        probe_command(
            FfmpegProbeKind::Amf,
            &[],
            None,
            &["-c:v", "h264_amf", "-quality", "speed"],
        ),
        probe_command(
            FfmpegProbeKind::Qsv,
            &[],
            None,
            &["-c:v", "h264_qsv", "-preset", "veryfast"],
        ),
    ];
    if cfg!(windows) {
        commands.push(probe_command(
            FfmpegProbeKind::MediaFoundation,
            &[],
            None,
            &["-c:v", "h264_mf", "-hw_encoding", "true"],
        ));
    }
    commands.extend([
        probe_command(
            FfmpegProbeKind::D3d11vaAmf,
            &["-init_hw_device", "d3d11va=gpu", "-filter_hw_device", "gpu"],
            Some("format=nv12,hwupload"),
            &["-c:v", "h264_amf", "-quality", "speed"],
        ),
        probe_command(
            FfmpegProbeKind::VulkanLibplacebo,
            &["-init_hw_device", "vulkan=gpu", "-filter_hw_device", "gpu"],
            Some("format=nv12,hwupload,libplacebo=w=320:h=240:brightness=0.001,gblur_vulkan=sigma=0.01,hwdownload,format=nv12"),
            &[],
        ),
        probe_command(
            FfmpegProbeKind::Cpu,
            &[],
            None,
            &["-c:v", "libopenh264", "-threads:v", "1"],
        ),
    ]);
    commands
}

pub fn evaluate_ffmpeg_capabilities(results: &[FfmpegProbeResult]) -> FfmpegGpuCapabilityReport {
    let d3d11va = capability_state(results, &[FfmpegProbeKind::D3d11vaAmf]);
    let vulkan_libplacebo = capability_state(results, &[FfmpegProbeKind::VulkanLibplacebo]);
    let mut amf = capability_state(results, &[FfmpegProbeKind::Amf]);
    if result_available(results, FfmpegProbeKind::D3d11vaAmf) {
        amf = FfmpegCapabilityState {
            probed: true,
            available: true,
            reason: None,
        };
    }
    let nvenc = capability_state(results, &[FfmpegProbeKind::Nvenc]);
    let qsv = capability_state(results, &[FfmpegProbeKind::Qsv]);
    let media_foundation = capability_state(results, &[FfmpegProbeKind::MediaFoundation]);
    let cpu = capability_state(results, &[FfmpegProbeKind::Cpu]);

    let selected_encoder_path = [
        (FfmpegProbeKind::Nvenc, FfmpegGpuPath::Nvenc),
        (FfmpegProbeKind::Amf, FfmpegGpuPath::Amf),
        (FfmpegProbeKind::Qsv, FfmpegGpuPath::Qsv),
        (
            FfmpegProbeKind::MediaFoundation,
            FfmpegGpuPath::MediaFoundation,
        ),
    ]
    .into_iter()
    .find_map(|(kind, path)| {
        (result_available(results, kind)
            || (kind == FfmpegProbeKind::Amf
                && result_available(results, FfmpegProbeKind::D3d11vaAmf)))
        .then_some(path)
    });
    let selected_encoder = selected_encoder_path
        .map(FfmpegGpuPath::encoder_name)
        .or_else(|| cpu.available.then_some("libopenh264"))
        .map(str::to_owned);
    let selected_gpu_path = selected_encoder_path.or_else(|| {
        result_available(results, FfmpegProbeKind::D3d11vaAmf)
            .then_some(FfmpegGpuPath::D3d11vaAmf)
            .or_else(|| {
                result_available(results, FfmpegProbeKind::VulkanLibplacebo)
                    .then_some(FfmpegGpuPath::VulkanLibplacebo)
            })
    });

    let fallback_reason = selected_gpu_path.is_none().then(|| {
        let reasons = results
            .iter()
            .filter(|result| result.kind != FfmpegProbeKind::Cpu && !result.available)
            .map(|result| {
                format!(
                    "{}: {}",
                    result.kind.label(),
                    result.reason.as_deref().unwrap_or("探测失败（无详细原因）")
                )
            })
            .collect::<Vec<_>>();
        if reasons.is_empty() {
            "未执行 GPU 闭环探测".to_owned()
        } else {
            reasons.join("；")
        }
    });

    FfmpegGpuCapabilityReport {
        d3d11va,
        vulkan_libplacebo,
        amf,
        nvenc,
        qsv,
        media_foundation,
        cpu,
        selected_encoder,
        selected_backend: if selected_gpu_path.is_some() {
            FfmpegRenderBackend::FfmpegGpu
        } else {
            FfmpegRenderBackend::FfmpegCpu
        },
        selected_gpu_path,
        fallback_reason,
    }
}

impl FfmpegProbeKind {
    fn label(self) -> &'static str {
        match self {
            Self::D3d11vaAmf => "d3d11va_amf",
            Self::VulkanLibplacebo => "vulkan_libplacebo",
            Self::Amf => "amf",
            Self::Nvenc => "nvenc",
            Self::Qsv => "qsv",
            Self::MediaFoundation => "media_foundation",
            Self::Cpu => "cpu",
        }
    }
}

impl FfmpegGpuPath {
    fn encoder_name(self) -> &'static str {
        match self {
            Self::Nvenc => "h264_nvenc",
            Self::Amf | Self::D3d11vaAmf => "h264_amf",
            Self::Qsv => "h264_qsv",
            Self::MediaFoundation => "h264_mf",
            Self::VulkanLibplacebo => "libopenh264",
        }
    }
}

fn probe_command(
    kind: FfmpegProbeKind,
    device_args: &[&str],
    video_filter: Option<&str>,
    encoder_args: &[&str],
) -> FfmpegProbeCommand {
    let mut args = ["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]
        .into_iter()
        .chain(device_args.iter().copied())
        .chain([
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x240:r=30:d=0.1,format=yuv420p",
            "-frames:v",
            "3",
            "-an",
        ])
        .map(OsString::from)
        .collect::<Vec<_>>();
    if let Some(video_filter) = video_filter {
        args.extend([OsString::from("-vf"), OsString::from(video_filter)]);
    }
    args.extend(encoder_args.iter().copied().map(OsString::from));
    args.extend([
        OsString::from("-f"),
        OsString::from("null"),
        OsString::from(if cfg!(windows) { "NUL" } else { "/dev/null" }),
    ]);
    FfmpegProbeCommand { kind, args }
}

fn capability_state(
    results: &[FfmpegProbeResult],
    kinds: &[FfmpegProbeKind],
) -> FfmpegCapabilityState {
    let matching = results
        .iter()
        .filter(|result| kinds.contains(&result.kind))
        .collect::<Vec<_>>();
    if matching.iter().any(|result| result.available) {
        return FfmpegCapabilityState {
            probed: true,
            available: true,
            reason: None,
        };
    }
    FfmpegCapabilityState {
        probed: !matching.is_empty(),
        available: false,
        reason: (!matching.is_empty()).then(|| {
            matching
                .iter()
                .map(|result| result.reason.as_deref().unwrap_or("探测失败（无详细原因）"))
                .collect::<Vec<_>>()
                .join("；")
        }),
    }
}

fn result_available(results: &[FfmpegProbeResult], kind: FfmpegProbeKind) -> bool {
    results
        .iter()
        .any(|result| result.kind == kind && result.available)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(command: &FfmpegProbeCommand) -> Vec<String> {
        command
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect()
    }

    fn command(kind: FfmpegProbeKind) -> FfmpegProbeCommand {
        ffmpeg_probe_commands()
            .into_iter()
            .find(|command| command.kind == kind)
            .expect("probe command must exist")
    }

    #[test]
    fn probes_run_real_short_samples_instead_of_listing_encoder_names() {
        let commands = ffmpeg_probe_commands();
        assert_eq!(commands.len(), if cfg!(windows) { 7 } else { 6 });

        for command in &commands {
            let args = args(command);
            assert!(args.windows(2).any(|pair| pair == ["-f", "lavfi"]));
            assert!(args.windows(2).any(|pair| pair == ["-f", "null"]));
            assert!(args
                .iter()
                .any(|arg| arg == if cfg!(windows) { "NUL" } else { "/dev/null" }));
            assert!(!args.iter().any(|arg| arg == "-encoders"));
        }

        let d3d11va = args(&command(FfmpegProbeKind::D3d11vaAmf));
        assert!(d3d11va
            .windows(2)
            .any(|pair| pair == ["-init_hw_device", "d3d11va=gpu"]));
        assert!(d3d11va.iter().any(|arg| arg.contains("hwupload")));
        assert!(d3d11va.windows(2).any(|pair| pair == ["-c:v", "h264_amf"]));

        let vulkan = args(&command(FfmpegProbeKind::VulkanLibplacebo));
        assert!(vulkan
            .windows(2)
            .any(|pair| pair == ["-init_hw_device", "vulkan=gpu"]));
        assert!(vulkan.iter().any(|arg| arg.contains("libplacebo=")));
        assert!(vulkan.iter().any(|arg| arg.contains("hwdownload")));
        assert!(!vulkan.iter().any(|arg| arg == "h264_amf"));

        if cfg!(windows) {
            let media_foundation = args(&command(FfmpegProbeKind::MediaFoundation));
            assert!(media_foundation
                .windows(2)
                .any(|pair| pair == ["-c:v", "h264_mf"]));
            assert!(media_foundation
                .windows(2)
                .any(|pair| pair == ["-hw_encoding", "true"]));
        }
    }

    #[test]
    fn fixed_encoder_order_does_not_depend_on_gpu_vendor_names() {
        let encoder_probe_order = ffmpeg_probe_commands()
            .into_iter()
            .filter_map(|probe| match probe.kind {
                FfmpegProbeKind::Nvenc
                | FfmpegProbeKind::Amf
                | FfmpegProbeKind::Qsv
                | FfmpegProbeKind::MediaFoundation
                | FfmpegProbeKind::Cpu => Some(probe.kind),
                FfmpegProbeKind::D3d11vaAmf | FfmpegProbeKind::VulkanLibplacebo => None,
            })
            .collect::<Vec<_>>();
        let expected = if cfg!(windows) {
            vec![
                FfmpegProbeKind::Nvenc,
                FfmpegProbeKind::Amf,
                FfmpegProbeKind::Qsv,
                FfmpegProbeKind::MediaFoundation,
                FfmpegProbeKind::Cpu,
            ]
        } else {
            vec![
                FfmpegProbeKind::Nvenc,
                FfmpegProbeKind::Amf,
                FfmpegProbeKind::Qsv,
                FfmpegProbeKind::Cpu,
            ]
        };
        assert_eq!(encoder_probe_order, expected);

        let results = [
            FfmpegProbeResult::available(FfmpegProbeKind::Qsv),
            FfmpegProbeResult::available(FfmpegProbeKind::Nvenc),
            FfmpegProbeResult::available(FfmpegProbeKind::Amf),
        ];
        let report = evaluate_ffmpeg_capabilities(&results);
        assert_eq!(report.selected_backend, FfmpegRenderBackend::FfmpegGpu);
        assert_eq!(report.selected_gpu_path, Some(FfmpegGpuPath::Nvenc));
        assert_eq!(report.selected_encoder.as_deref(), Some("h264_nvenc"));
    }

    #[test]
    fn vendor_neutral_vulkan_probe_does_not_claim_amf() {
        let report = evaluate_ffmpeg_capabilities(&[FfmpegProbeResult::available(
            FfmpegProbeKind::VulkanLibplacebo,
        )]);
        assert!(report.vulkan_libplacebo.available);
        assert!(!report.amf.available);
        assert_eq!(
            report.selected_gpu_path,
            Some(FfmpegGpuPath::VulkanLibplacebo)
        );
        assert_eq!(report.selected_encoder, None);
    }

    #[test]
    fn vulkan_filter_capability_is_reused_with_each_vendor_encoder_probe() {
        for (encoder, expected_path, expected_name) in [
            (FfmpegProbeKind::Amf, FfmpegGpuPath::Amf, "h264_amf"),
            (FfmpegProbeKind::Nvenc, FfmpegGpuPath::Nvenc, "h264_nvenc"),
            (FfmpegProbeKind::Qsv, FfmpegGpuPath::Qsv, "h264_qsv"),
        ] {
            let report = evaluate_ffmpeg_capabilities(&[
                FfmpegProbeResult::available(FfmpegProbeKind::VulkanLibplacebo),
                FfmpegProbeResult::available(encoder),
            ]);
            assert!(report.vulkan_libplacebo.available);
            assert_eq!(report.selected_gpu_path, Some(expected_path));
            assert_eq!(report.selected_encoder.as_deref(), Some(expected_name));
        }
    }

    #[test]
    fn amf_is_available_only_after_a_real_amf_probe_succeeds() {
        let report = evaluate_ffmpeg_capabilities(&[FfmpegProbeResult::unavailable(
            FfmpegProbeKind::Amf,
            "AMF process exited with code 1",
        )]);
        assert!(report.amf.probed);
        assert!(!report.amf.available);
        assert_eq!(report.selected_backend, FfmpegRenderBackend::FfmpegCpu);
        assert_eq!(report.selected_encoder, None);
    }

    #[test]
    fn all_gpu_probe_failures_fall_back_to_cpu_with_observable_reason() {
        let results = [
            FfmpegProbeResult::unavailable(FfmpegProbeKind::D3d11vaAmf, "D3D11VA unavailable"),
            FfmpegProbeResult::unavailable(FfmpegProbeKind::VulkanLibplacebo, "Vulkan unavailable"),
            FfmpegProbeResult::unavailable(FfmpegProbeKind::Amf, "AMF unavailable"),
            FfmpegProbeResult::unavailable(FfmpegProbeKind::Nvenc, "NVENC unavailable"),
            FfmpegProbeResult::unavailable(FfmpegProbeKind::Qsv, "QSV unavailable"),
            FfmpegProbeResult::unavailable(
                FfmpegProbeKind::MediaFoundation,
                "Media Foundation unavailable",
            ),
            FfmpegProbeResult::available(FfmpegProbeKind::Cpu),
        ];
        let report = evaluate_ffmpeg_capabilities(&results);
        assert_eq!(report.selected_backend, FfmpegRenderBackend::FfmpegCpu);
        assert_eq!(report.selected_gpu_path, None);
        assert_eq!(report.selected_encoder.as_deref(), Some("libopenh264"));
        let reason = report.fallback_reason.expect("fallback reason");
        assert!(reason.contains("D3D11VA unavailable"));
        assert!(reason.contains("Vulkan unavailable"));
        assert!(reason.contains("AMF unavailable"));
        assert!(reason.contains("NVENC unavailable"));
        assert!(reason.contains("QSV unavailable"));
        assert!(reason.contains("Media Foundation unavailable"));
    }
}
