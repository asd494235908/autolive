use serde::{Deserialize, Serialize};

pub const FULL_GPU_EFFECT_COUNT: u16 = 83;

/// 与厂商无关的实时视频候选路径，顺序即默认探测/降级顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeVideoCandidate {
    D3d11ZeroCopy,
    D3d11Copy,
    VulkanWinvkCopy,
    GpuSoftwareDecode,
    Cpu4,
    Original,
}

impl RealtimeVideoCandidate {
    pub const GPU_ORDER: [Self; 4] = [
        Self::D3d11ZeroCopy,
        Self::D3d11Copy,
        Self::VulkanWinvkCopy,
        Self::GpuSoftwareDecode,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeVideoEffectTier {
    FullGpu83,
    Cpu4,
    Original,
}

/// 能稳定区分物理/虚拟适配器和驱动版本的缓存身份；不参与厂商分支判断。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuAdapterIdentity {
    pub name: String,
    pub vendor_id: Option<u32>,
    pub device_id: Option<u32>,
    pub luid: Option<String>,
    pub driver_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeVideoCapabilityCacheKey {
    pub adapter: GpuAdapterIdentity,
    pub mpv_version: String,
    pub libplacebo_version: String,
    pub shader_sha256: String,
}

/// 单条 GPU 候选的纯探测结果；本模块不负责执行硬件或 mpv 探测。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuCandidateProbe {
    pub candidate: RealtimeVideoCandidate,
    pub supported_effects: u16,
    pub device_ready: bool,
    pub decode_ready: bool,
    pub render_ready: bool,
    pub shader_ready: bool,
    pub history_frame_ready: bool,
    pub scheduler_ready: bool,
}

impl GpuCandidateProbe {
    pub fn supports_full_gpu83(&self) -> bool {
        RealtimeVideoCandidate::GPU_ORDER.contains(&self.candidate)
            && self.supported_effects == FULL_GPU_EFFECT_COUNT
            && self.device_ready
            && self.decode_ready
            && self.render_ready
            && self.shader_ready
            && self.history_frame_ready
            && self.scheduler_ready
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeVideoCapabilityReport {
    pub gpu_candidates: Vec<GpuCandidateProbe>,
    pub cpu4_ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeVideoSelection {
    pub candidate: RealtimeVideoCandidate,
    pub effect_tier: RealtimeVideoEffectTier,
}

/// 严格按固定顺序选择；GPU 路径必须完整通过 83/83 门禁，不做部分参数运行。
pub fn select_realtime_video_candidate(
    report: &RealtimeVideoCapabilityReport,
) -> RealtimeVideoSelection {
    for candidate in RealtimeVideoCandidate::GPU_ORDER {
        if report
            .gpu_candidates
            .iter()
            .filter(|probe| probe.candidate == candidate)
            .any(GpuCandidateProbe::supports_full_gpu83)
        {
            return RealtimeVideoSelection {
                candidate,
                effect_tier: RealtimeVideoEffectTier::FullGpu83,
            };
        }
    }

    if report.cpu4_ready {
        RealtimeVideoSelection {
            candidate: RealtimeVideoCandidate::Cpu4,
            effect_tier: RealtimeVideoEffectTier::Cpu4,
        }
    } else {
        RealtimeVideoSelection {
            candidate: RealtimeVideoCandidate::Original,
            effect_tier: RealtimeVideoEffectTier::Original,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_probe(candidate: RealtimeVideoCandidate) -> GpuCandidateProbe {
        GpuCandidateProbe {
            candidate,
            supported_effects: FULL_GPU_EFFECT_COUNT,
            device_ready: true,
            decode_ready: true,
            render_ready: true,
            shader_ready: true,
            history_frame_ready: true,
            scheduler_ready: true,
        }
    }

    fn report(probes: Vec<GpuCandidateProbe>, cpu4_ready: bool) -> RealtimeVideoCapabilityReport {
        RealtimeVideoCapabilityReport {
            gpu_candidates: probes,
            cpu4_ready,
        }
    }

    #[test]
    fn selects_first_candidate_that_passes_all_83_effects() {
        let selection = select_realtime_video_candidate(&report(
            vec![passing_probe(RealtimeVideoCandidate::D3d11ZeroCopy)],
            true,
        ));

        assert_eq!(
            selection,
            RealtimeVideoSelection {
                candidate: RealtimeVideoCandidate::D3d11ZeroCopy,
                effect_tier: RealtimeVideoEffectTier::FullGpu83,
            }
        );
    }

    #[test]
    fn rejects_candidate_with_only_82_effects() {
        let mut incomplete = passing_probe(RealtimeVideoCandidate::D3d11ZeroCopy);
        incomplete.supported_effects = FULL_GPU_EFFECT_COUNT - 1;

        let selection = select_realtime_video_candidate(&report(
            vec![incomplete, passing_probe(RealtimeVideoCandidate::D3d11Copy)],
            true,
        ));

        assert_eq!(selection.candidate, RealtimeVideoCandidate::D3d11Copy);
        assert_eq!(selection.effect_tier, RealtimeVideoEffectTier::FullGpu83);
    }

    #[test]
    fn rejects_candidate_when_history_frame_probe_fails() {
        let mut no_history = passing_probe(RealtimeVideoCandidate::D3d11ZeroCopy);
        no_history.history_frame_ready = false;

        let selection = select_realtime_video_candidate(&report(
            vec![
                no_history,
                passing_probe(RealtimeVideoCandidate::VulkanWinvkCopy),
            ],
            true,
        ));

        assert_eq!(selection.candidate, RealtimeVideoCandidate::VulkanWinvkCopy);
        assert_eq!(selection.effect_tier, RealtimeVideoEffectTier::FullGpu83);
    }

    #[test]
    fn falls_back_to_cpu4_when_all_gpu_candidates_fail() {
        let failed = RealtimeVideoCandidate::GPU_ORDER
            .into_iter()
            .map(|candidate| {
                let mut probe = passing_probe(candidate);
                probe.render_ready = false;
                probe
            })
            .collect();

        let selection = select_realtime_video_candidate(&report(failed, true));

        assert_eq!(selection.candidate, RealtimeVideoCandidate::Cpu4);
        assert_eq!(selection.effect_tier, RealtimeVideoEffectTier::Cpu4);
    }

    #[test]
    fn falls_back_to_original_when_cpu4_also_fails() {
        let selection = select_realtime_video_candidate(&report(Vec::new(), false));

        assert_eq!(selection.candidate, RealtimeVideoCandidate::Original);
        assert_eq!(selection.effect_tier, RealtimeVideoEffectTier::Original);
    }

    #[test]
    fn a_stale_failed_probe_does_not_hide_a_later_passing_probe_for_the_same_candidate() {
        let mut stale = passing_probe(RealtimeVideoCandidate::D3d11ZeroCopy);
        stale.shader_ready = false;
        let selection = select_realtime_video_candidate(&report(
            vec![stale, passing_probe(RealtimeVideoCandidate::D3d11ZeroCopy)],
            true,
        ));

        assert_eq!(selection.candidate, RealtimeVideoCandidate::D3d11ZeroCopy);
        assert_eq!(selection.effect_tier, RealtimeVideoEffectTier::FullGpu83);
    }
}
