//! 离线普通声音的补充效果规划。
//!
//! 能由 FFmpeg 忠实表达的效果生成滤镜或双输入 complex-graph 计划；需要 PCM
//! 分析/重建的 MFCC、SNR 和共振峰参数则生成明确的运行时配置。规划不会创建外部
//! 环境声，也不会启动实时话术、speech-to-speech 或模型链路。

use crate::media_effect_params::{AudioEffectParams, NaturalVoiceMode, ParameterValidationError};

const DEFAULT_MFCC_DIMENSIONS: u8 = 13;
const DEFAULT_SPECTRUM_BLIND_SPOT_CENTER_HZ: f64 = 8_000.0;
const SPECTRUM_REFERENCE_MAX_HZ: f64 = 20_000.0;
const SNR_ANALYSIS_WINDOW_MS: u64 = 400;
const SNR_UPDATE_INTERVAL_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct AudioPresetController {
    pub natural_voice_mode: NaturalVoiceMode,
    pub voice_library_id: Option<String>,
    /// 可直接接入普通声音 FFmpeg 支路的本地预设滤镜。
    pub serial_filters: Vec<String>,
    /// 自然动态模式按该媒体时间周期更新；`None` 表示不需要周期控制。
    pub dynamic_period_ms: Option<u64>,
    /// 音色 ID 解析出的内建、确定性预设参数；不读取或生成话术音轨。
    pub voice_preset: Option<VoicePresetFilterParams>,
}

impl AudioPresetController {
    pub fn requires_dynamic_scheduler(&self) -> bool {
        self.natural_voice_mode == NaturalVoiceMode::NaturalDynamic
    }

    pub fn contributes_dsp(&self) -> bool {
        !self.serial_filters.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoicePresetFilterParams {
    pub stable_seed: u64,
    pub center_frequency_hz: f64,
    pub gain_db: f64,
    pub q: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxiliaryMixSource {
    /// 调用方提供并已校验的外部环境声音频支路。
    AmbientSound,
    /// 调用方已有的完整湿声处理支路。
    ProcessedWet,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuxiliaryMixPlan {
    pub source: AuxiliaryMixSource,
    pub dry_weight: f64,
    pub auxiliary_weight: f64,
}

impl AuxiliaryMixPlan {
    /// 调用方以 `[dry][auxiliary]` 顺序连接两条真实音频支路。
    pub fn ffmpeg_filter(&self) -> String {
        format!(
            "amix=inputs=2:weights={:.6} {:.6}:duration=first:dropout_transition=0:normalize=0",
            self.dry_weight, self.auxiliary_weight
        )
    }

    /// 标签由媒体引擎分配；该片段不会自行制造第二输入。
    pub fn ffmpeg_complex_graph_fragment(
        &self,
        dry_input_label: &str,
        auxiliary_input_label: &str,
        output_label: &str,
    ) -> String {
        format!(
            "[{dry_input_label}][{auxiliary_input_label}]{}[{output_label}]",
            self.ffmpeg_filter()
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpectrumBlindSpotShape {
    BandReject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpectrumBlindSpotPlan {
    pub center_frequency_hz: f64,
    pub bandwidth_hz: f64,
    pub shape: SpectrumBlindSpotShape,
}

impl SpectrumBlindSpotPlan {
    pub fn ffmpeg_filter(&self) -> String {
        match self.shape {
            SpectrumBlindSpotShape::BandReject => format!(
                "bandreject=f={:.6}:t=h:w={:.6}",
                self.center_frequency_hz, self.bandwidth_hz
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnrTargetPlan {
    /// 首个分析窗口测得的源素材基线；后续只应用 `variation_db`。
    MeasuredSourceBaseline,
    FixedDb(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnrProcessingBackend {
    /// PCM 处理器滚动测量信号 RMS，并按目标注入有界噪声；静态 FFmpeg
    /// 滤镜无法在未知输入电平下保证目标 SNR。
    PcmRollingRmsNoiseInjection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnrRuntimePlan {
    pub target: SnrTargetPlan,
    /// 应用到固定/实测基线后的有符号 dB 偏移。
    pub variation_db: f64,
    pub analysis_window_ms: u64,
    pub update_interval_ms: u64,
    pub backend: SnrProcessingBackend,
}

impl SnrRuntimePlan {
    pub fn target_db_for_measured_source(&self, measured_source_snr_db: f64) -> f64 {
        let baseline_db = match self.target {
            SnrTargetPlan::MeasuredSourceBaseline => measured_source_snr_db,
            SnrTargetPlan::FixedDb(target_db) => target_db,
        };
        (baseline_db + self.variation_db).clamp(0.0, 60.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MfccOperation {
    /// 仅提取真实特征，不改变 PCM。
    Analyze,
    /// 必须经过特征修改和可逆重建/声码器；仅提取特征不算生效。
    ShiftAndReconstruct,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MfccRuntimePlan {
    pub dimensions: u8,
    pub shift_percent: f64,
    pub operation: MfccOperation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioAnalysisRuntimePlan {
    /// 由独立 PCM MFCC 内核消费；不能用 EQ 替代特征重建。
    pub mfcc: Option<MfccRuntimePlan>,
    /// 由共振峰分析内核产生/校验的测量值，不作为 FFmpeg DSP 输入。
    pub current_formant_hz: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfflineAudioEffectPlan {
    /// 可直接追加到单输入 FFmpeg 音频链的滤镜。
    pub serial_filters: Vec<String>,
    /// 原声与完整湿声支路的比例混合；调用方必须真实提供湿声支路。
    pub dry_wet_mix: Option<AuxiliaryMixPlan>,
    /// 主声音与外部环境声支路的比例混合；调用方必须真实提供环境声素材。
    pub ambient_sound_mix: Option<AuxiliaryMixPlan>,
    /// 本地普通声音预设与周期控制器；不启动话术模型或候选话术音轨。
    pub preset_controller: AudioPresetController,
    /// 固定中心与带阻形状均显式暴露，后续契约补字段时可直接替换默认值。
    pub spectrum_blind_spot: Option<SpectrumBlindSpotPlan>,
    /// 由混音后 PCM 运行时消费的真实 SNR 配置。
    pub snr: Option<SnrRuntimePlan>,
    /// 由独立 PCM 特征分析/重建内核消费的配置。
    pub analysis: AudioAnalysisRuntimePlan,
}

impl OfflineAudioEffectPlan {
    pub fn required_ffmpeg_filters(&self) -> Vec<&'static str> {
        let mut required = Vec::with_capacity(4);
        if !self.serial_filters.is_empty() {
            if self
                .serial_filters
                .iter()
                .any(|filter| filter.starts_with("afftfilt="))
            {
                required.push("afftfilt");
            }
            if self
                .serial_filters
                .iter()
                .any(|filter| filter.starts_with("volume="))
            {
                required.push("volume");
            }
            if self
                .serial_filters
                .iter()
                .any(|filter| filter.starts_with("equalizer="))
            {
                required.push("equalizer");
            }
            if self
                .serial_filters
                .iter()
                .any(|filter| filter.starts_with("bandreject="))
            {
                required.push("bandreject");
            }
        }
        if self.dry_wet_mix.is_some() || self.ambient_sound_mix.is_some() {
            required.push("amix");
        }
        required
    }
}

pub fn build_offline_audio_effect_plan(
    audio: &AudioEffectParams,
) -> Result<OfflineAudioEffectPlan, Vec<ParameterValidationError>> {
    audio.validate()?;

    let preset_controller = audio_preset_controller(audio);
    let mut serial_filters = preset_controller.serial_filters.clone();
    serial_filters.extend(spectral_perturbation_filter(
        audio.spectral_perturbation_percent,
    ));
    if let Some(filter) = high_frequency_perturbation_filter(audio) {
        serial_filters.push(filter);
    }
    let spectrum_blind_spot = spectrum_blind_spot_plan(audio.spectrum_blind_spot_percent);
    if let Some(plan) = &spectrum_blind_spot {
        serial_filters.push(plan.ffmpeg_filter());
    }
    let dry_wet_mix = auxiliary_mix(AuxiliaryMixSource::ProcessedWet, audio.dry_wet_percent);
    let ambient_sound_mix = auxiliary_mix(
        AuxiliaryMixSource::AmbientSound,
        audio.ambient_sound_mix_percent,
    );
    let snr = snr_runtime_plan(audio);
    let analysis = analysis_runtime_plan(audio);

    Ok(OfflineAudioEffectPlan {
        serial_filters,
        dry_wet_mix,
        ambient_sound_mix,
        preset_controller,
        spectrum_blind_spot,
        snr,
        analysis,
    })
}

fn audio_preset_controller(audio: &AudioEffectParams) -> AudioPresetController {
    let mut serial_filters = Vec::with_capacity(2);
    let dynamic_period_ms = (audio.natural_voice_mode == NaturalVoiceMode::NaturalDynamic)
        .then_some(audio.random_change_period_ms);
    if let Some(period_ms) = dynamic_period_ms {
        let period_seconds = period_ms as f64 / 1_000.0;
        // 1.2% 的慢速包络只改变普通声音的局部响度，不修改话术或音频时长。
        serial_filters.push(format!(
            "volume='1+0.012000*sin(2*PI*t/{period_seconds:.6})':eval=frame"
        ));
    }

    let voice_preset = audio.voice_library_id.as_deref().map(voice_preset_params);
    if let Some(preset) = &voice_preset {
        serial_filters.push(format!(
            "equalizer=f={:.6}:t=q:w={:.6}:g={:.6}",
            preset.center_frequency_hz, preset.q, preset.gain_db
        ));
    }

    AudioPresetController {
        natural_voice_mode: audio.natural_voice_mode,
        voice_library_id: audio.voice_library_id.clone(),
        serial_filters,
        dynamic_period_ms,
        voice_preset,
    }
}

fn voice_preset_params(voice_library_id: &str) -> VoicePresetFilterParams {
    const CENTER_FREQUENCIES_HZ: [f64; 8] = [
        180.0, 260.0, 420.0, 700.0, 1_100.0, 1_700.0, 2_600.0, 3_600.0,
    ];

    // FNV-1a 保证同一音色 ID 在各平台产生同一套轻量本地预设，不依赖随机状态。
    let stable_seed = voice_library_id
        .as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    let center_frequency_hz =
        CENTER_FREQUENCIES_HZ[(stable_seed as usize) % CENTER_FREQUENCIES_HZ.len()];
    let magnitude_db = 0.5 + ((stable_seed >> 8) % 5) as f64 * 0.25;
    let gain_db = if stable_seed & 1 == 0 {
        magnitude_db
    } else {
        -magnitude_db
    };
    let q = 0.8 + ((stable_seed >> 16) % 5) as f64 * 0.2;

    VoicePresetFilterParams {
        stable_seed,
        center_frequency_hz,
        gain_db,
        q,
    }
}

fn spectrum_blind_spot_plan(percent: f64) -> Option<SpectrumBlindSpotPlan> {
    (percent > f64::EPSILON).then_some(SpectrumBlindSpotPlan {
        center_frequency_hz: DEFAULT_SPECTRUM_BLIND_SPOT_CENTER_HZ,
        bandwidth_hz: SPECTRUM_REFERENCE_MAX_HZ * percent / 100.0,
        shape: SpectrumBlindSpotShape::BandReject,
    })
}

fn snr_runtime_plan(audio: &AudioEffectParams) -> Option<SnrRuntimePlan> {
    if audio.snr_target_db.is_none() && audio.snr_variation_db.abs() <= f64::EPSILON {
        return None;
    }

    Some(SnrRuntimePlan {
        target: audio.snr_target_db.map_or(
            SnrTargetPlan::MeasuredSourceBaseline,
            SnrTargetPlan::FixedDb,
        ),
        variation_db: audio.snr_variation_db,
        analysis_window_ms: SNR_ANALYSIS_WINDOW_MS,
        update_interval_ms: SNR_UPDATE_INTERVAL_MS,
        backend: SnrProcessingBackend::PcmRollingRmsNoiseInjection,
    })
}

fn analysis_runtime_plan(audio: &AudioEffectParams) -> AudioAnalysisRuntimePlan {
    let mfcc = (audio.mfcc_shift_percent.abs() > f64::EPSILON
        || audio.mfcc_dimensions != DEFAULT_MFCC_DIMENSIONS)
        .then_some(MfccRuntimePlan {
            dimensions: audio.mfcc_dimensions,
            shift_percent: audio.mfcc_shift_percent,
            operation: if audio.mfcc_shift_percent.abs() > f64::EPSILON {
                MfccOperation::ShiftAndReconstruct
            } else {
                MfccOperation::Analyze
            },
        });

    AudioAnalysisRuntimePlan {
        mfcc,
        current_formant_hz: audio.current_formant_hz,
    }
}

fn spectral_perturbation_filter(percent: f64) -> Option<String> {
    if percent <= f64::EPSILON {
        return None;
    }
    let amplitude = percent / 100.0;
    // 实部和虚部使用同一有界倍率，改变频谱幅度但不额外旋转相位。
    let scale = format!("1+{amplitude:.6}*sin(2*PI*b/nb*7+ch*PI/3)");
    Some(format!(
        "afftfilt=real='re*({scale})':imag='im*({scale})':win_size=4096:win_func=hann:overlap=0.75"
    ))
}

fn high_frequency_perturbation_filter(audio: &AudioEffectParams) -> Option<String> {
    if !audio.high_frequency_perturbation_enabled
        || audio.high_frequency_perturbation_strength_percent <= f64::EPSILON
    {
        return None;
    }
    let interval_seconds = audio.high_frequency_perturbation_interval_ms as f64 / 1_000.0;
    let level = 10_f64.powf(audio.high_frequency_perturbation_level_db / 20.0);
    let amplitude = audio.high_frequency_perturbation_strength_percent / 100.0 * level;
    // `b/nb` 是当前 FFT bin 在 0..Nyquist 的归一化位置；0.25 对应
    // 48kHz 总线的 6kHz。总线在进入该链前会被归一到 44.1/48kHz，
    // 这里使用保守阈值，避免扰动主要语音基频与低频共振峰。
    let scale = format!("1+gte(b/nb\\,0.25)*{amplitude:.8}*sin(2*PI*b/nb*11+ch*PI/5)");
    Some(format!(
        "afftfilt=real='re*({scale})':imag='im*({scale})':win_size=4096:win_func=hann:overlap=0.75:enable='lt(mod(t\\,{interval_seconds:.6})\\,{active_seconds:.6})'",
        active_seconds = (interval_seconds / 2.0).max(0.25),
    ))
}

fn auxiliary_mix(source: AuxiliaryMixSource, percent: f64) -> Option<AuxiliaryMixPlan> {
    if percent <= f64::EPSILON {
        return None;
    }
    let auxiliary_weight = percent / 100.0;
    Some(AuxiliaryMixPlan {
        source,
        dry_weight: 1.0 - auxiliary_weight,
        auxiliary_weight,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_offline_audio_effect_plan, AuxiliaryMixSource, MfccOperation, SnrProcessingBackend,
        SnrTargetPlan, SpectrumBlindSpotShape,
    };
    use crate::media_effect_params::{AudioEffectParams, NaturalVoiceMode};
    use std::process::{Command, Stdio};

    #[test]
    fn defaults_request_no_extra_effect_or_fake_measurement() {
        let plan = build_offline_audio_effect_plan(&AudioEffectParams::default())
            .expect("defaults should build");

        assert!(plan.serial_filters.is_empty());
        assert!(plan.dry_wet_mix.is_none());
        assert!(plan.ambient_sound_mix.is_none());
        assert!(plan.spectrum_blind_spot.is_none());
        assert!(plan.snr.is_none());
        assert!(plan.analysis.mfcc.is_none());
        assert!(plan.analysis.current_formant_hz.is_none());
        assert!(plan.required_ffmpeg_filters().is_empty());
        assert!(!plan.preset_controller.requires_dynamic_scheduler());
        assert!(!plan.preset_controller.contributes_dsp());
    }

    #[test]
    fn spectral_perturbation_is_a_bounded_real_ffmpeg_fft_filter() {
        let audio = AudioEffectParams {
            spectral_perturbation_percent: 10.0,
            ..Default::default()
        };
        let plan = build_offline_audio_effect_plan(&audio).expect("valid spectral effect");
        let filter = plan.serial_filters.first().expect("spectral filter");

        assert!(filter.starts_with("afftfilt="));
        assert!(filter.contains("1+0.100000*sin("));
        assert!(filter.contains("real='re*("));
        assert!(filter.contains("imag='im*("));
        assert_eq!(plan.required_ffmpeg_filters(), vec!["afftfilt"]);
    }

    #[test]
    fn high_frequency_perturbation_is_gated_and_stays_above_voice_band() {
        let audio = AudioEffectParams {
            high_frequency_perturbation_enabled: true,
            high_frequency_perturbation_interval_ms: 12_000,
            high_frequency_perturbation_strength_percent: 4.0,
            high_frequency_perturbation_level_db: -32.0,
            ..Default::default()
        };
        let plan = build_offline_audio_effect_plan(&audio).expect("valid high-frequency effect");
        let filter = plan.serial_filters.first().expect("high-frequency filter");

        assert!(filter.contains("gte(b/nb\\,0.25)"));
        assert!(filter.contains("enable='lt(mod(t\\,12.000000)\\,6.000000)'"));
        assert_eq!(plan.required_ffmpeg_filters(), vec!["afftfilt"]);
    }

    #[test]
    fn packaged_ffmpeg_accepts_serial_supplementary_filters() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let audio = AudioEffectParams {
            natural_voice_mode: NaturalVoiceMode::NaturalDynamic,
            voice_library_id: Some("voice-local-1".to_owned()),
            spectrum_blind_spot_percent: 2.0,
            high_frequency_perturbation_enabled: true,
            high_frequency_perturbation_strength_percent: 4.0,
            ..Default::default()
        };
        let plan = build_offline_audio_effect_plan(&audio).expect("supplementary audio plan");
        let filter = plan.serial_filters.join(",");
        let output = Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=8000:sample_rate=48000:duration=0.25",
                "-af",
                &filter,
                "-f",
                "null",
                "-",
            ])
            .stdin(Stdio::null())
            .output()
            .expect("run packaged ffmpeg");

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn auxiliary_mix_requires_real_second_inputs_and_preserves_ratio() {
        let audio = AudioEffectParams {
            dry_wet_percent: 25.0,
            ambient_sound_mix_percent: 40.0,
            ..Default::default()
        };
        let plan = build_offline_audio_effect_plan(&audio).expect("valid mix ratios");
        let dry_wet = plan.dry_wet_mix.as_ref().expect("dry/wet mix");
        let ambient = plan.ambient_sound_mix.as_ref().expect("ambient mix");

        assert_eq!(dry_wet.source, AuxiliaryMixSource::ProcessedWet);
        assert_eq!(dry_wet.ffmpeg_filter(), "amix=inputs=2:weights=0.750000 0.250000:duration=first:dropout_transition=0:normalize=0");
        assert_eq!(ambient.source, AuxiliaryMixSource::AmbientSound);
        assert_eq!(ambient.ffmpeg_filter(), "amix=inputs=2:weights=0.600000 0.400000:duration=first:dropout_transition=0:normalize=0");
        assert_eq!(
            ambient.ffmpeg_complex_graph_fragment("main", "ambient", "mixed"),
            "[main][ambient]amix=inputs=2:weights=0.600000 0.400000:duration=first:dropout_transition=0:normalize=0[mixed]"
        );
        assert_eq!(plan.required_ffmpeg_filters(), vec!["amix"]);
    }

    #[test]
    fn mfcc_and_formant_fields_create_explicit_pcm_analysis_configuration() {
        let audio = AudioEffectParams {
            mfcc_shift_percent: 5.0,
            mfcc_dimensions: 20,
            current_formant_hz: Some(500.0),
            ..Default::default()
        };
        let plan = build_offline_audio_effect_plan(&audio).expect("valid formal parameters");
        let mfcc = plan.analysis.mfcc.expect("MFCC runtime plan");

        assert_eq!(mfcc.dimensions, 20);
        assert_eq!(mfcc.shift_percent, 5.0);
        assert_eq!(mfcc.operation, MfccOperation::ShiftAndReconstruct);
        assert_eq!(plan.analysis.current_formant_hz, Some(500.0));
        assert!(plan.serial_filters.is_empty());
    }

    #[test]
    fn mfcc_dimensions_alone_request_real_analysis_without_fake_dsp() {
        let audio = AudioEffectParams {
            mfcc_dimensions: 20,
            ..Default::default()
        };
        let plan = build_offline_audio_effect_plan(&audio).expect("valid MFCC analysis");
        let mfcc = plan.analysis.mfcc.expect("MFCC runtime plan");

        assert_eq!(mfcc.operation, MfccOperation::Analyze);
        assert!(plan.serial_filters.is_empty());
    }

    #[test]
    fn snr_fields_create_rolling_pcm_rms_and_noise_injection_plan() {
        let audio = AudioEffectParams {
            snr_variation_db: 1.0,
            snr_target_db: Some(30.0),
            ..Default::default()
        };
        let plan = build_offline_audio_effect_plan(&audio).expect("valid SNR parameters");
        let snr = plan.snr.expect("SNR runtime plan");

        assert_eq!(snr.target, SnrTargetPlan::FixedDb(30.0));
        assert_eq!(snr.variation_db, 1.0);
        assert_eq!(snr.target_db_for_measured_source(18.0), 31.0);
        assert_eq!(snr.analysis_window_ms, 400);
        assert_eq!(snr.update_interval_ms, 100);
        assert_eq!(
            snr.backend,
            SnrProcessingBackend::PcmRollingRmsNoiseInjection
        );
        assert!(plan.serial_filters.is_empty());
    }

    #[test]
    fn snr_variation_without_target_calibrates_from_measured_source() {
        let audio = AudioEffectParams {
            snr_variation_db: -2.0,
            ..Default::default()
        };
        let plan = build_offline_audio_effect_plan(&audio).expect("valid SNR variation");

        let snr = plan.snr.expect("SNR runtime plan");
        assert_eq!(snr.target, SnrTargetPlan::MeasuredSourceBaseline);
        assert_eq!(snr.target_db_for_measured_source(24.0), 22.0);
    }

    #[test]
    fn spectrum_blind_spot_uses_exposed_deterministic_center_and_shape() {
        let audio = AudioEffectParams {
            spectrum_blind_spot_percent: 2.0,
            ..Default::default()
        };
        let plan = build_offline_audio_effect_plan(&audio).expect("valid spectrum blind spot");
        let blind_spot = plan
            .spectrum_blind_spot
            .as_ref()
            .expect("spectrum blind spot plan");

        assert_eq!(blind_spot.center_frequency_hz, 8_000.0);
        assert_eq!(blind_spot.bandwidth_hz, 400.0);
        assert_eq!(blind_spot.shape, SpectrumBlindSpotShape::BandReject);
        assert_eq!(
            blind_spot.ffmpeg_filter(),
            "bandreject=f=8000.000000:t=h:w=400.000000"
        );
        assert!(plan.serial_filters.contains(&blind_spot.ffmpeg_filter()));
        assert_eq!(plan.required_ffmpeg_filters(), vec!["bandreject"]);
    }

    #[test]
    fn natural_mode_and_voice_library_each_produce_real_local_preset_filters() {
        let natural_audio = AudioEffectParams {
            natural_voice_mode: NaturalVoiceMode::NaturalDynamic,
            ..Default::default()
        };
        let natural_plan =
            build_offline_audio_effect_plan(&natural_audio).expect("valid natural controller");

        assert!(natural_plan.preset_controller.requires_dynamic_scheduler());
        assert_eq!(
            natural_plan.preset_controller.dynamic_period_ms,
            Some(4_000)
        );
        assert!(natural_plan.preset_controller.contributes_dsp());
        assert_eq!(natural_plan.preset_controller.serial_filters.len(), 1);
        assert!(natural_plan.preset_controller.serial_filters[0]
            .starts_with("volume='1+0.012000*sin(2*PI*t/4.000000)'"));
        assert_eq!(natural_plan.required_ffmpeg_filters(), vec!["volume"]);

        let voice_audio = AudioEffectParams {
            voice_library_id: Some("voice-local-1".to_owned()),
            ..Default::default()
        };
        let voice_plan = build_offline_audio_effect_plan(&voice_audio).expect("valid voice preset");
        let repeated_voice_plan =
            build_offline_audio_effect_plan(&voice_audio).expect("repeat voice preset");

        assert_eq!(
            voice_plan.preset_controller.voice_library_id.as_deref(),
            Some("voice-local-1")
        );
        assert!(!voice_plan.preset_controller.requires_dynamic_scheduler());
        assert!(voice_plan.preset_controller.voice_preset.is_some());
        assert!(voice_plan.preset_controller.contributes_dsp());
        assert_eq!(voice_plan.preset_controller.serial_filters.len(), 1);
        assert!(voice_plan.preset_controller.serial_filters[0].starts_with("equalizer="));
        assert_eq!(
            voice_plan.preset_controller.voice_preset,
            repeated_voice_plan.preset_controller.voice_preset
        );
        assert_eq!(voice_plan.required_ffmpeg_filters(), vec!["equalizer"]);
    }

    #[test]
    fn public_builder_keeps_parameter_validation_at_the_boundary() {
        let audio = AudioEffectParams {
            dry_wet_percent: 101.0,
            ..Default::default()
        };

        let errors = build_offline_audio_effect_plan(&audio).expect_err("invalid ratio");
        assert!(errors
            .iter()
            .any(|error| error.field == "audio.dry_wet_percent"));
    }
}
