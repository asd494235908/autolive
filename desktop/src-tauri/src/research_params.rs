//! 本地研究参数契约。
//!
//! 本模块只描述可序列化的参数、默认值和边界校验，不执行媒体处理，也不生成
//! 随机值。字段名中的 `_ms`、`_hz`、`_px`、`_percent` 等后缀是机器可读的单位
//! 语义；研究参数的实际算法、版本和随机种子由后续处理阶段另行记录。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 视频视觉调制实验固定使用的频段集合，单位为 Hz。
pub const VISUAL_BAND_FREQUENCIES_HZ: [u32; 12] = [
    65, 92, 131, 188, 267, 381, 544, 777, 1110, 1585, 2263, 20_000,
];

/// 自然真人模式枚举；模式本身没有物理单位。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NaturalVoiceMode {
    /// 保持源音频，不做变声实验。
    #[default]
    Original,
    /// 允许后续本地处理器按周期产生自然动态参数。
    NaturalDynamic,
}

/// 单个参数校验失败的结构化描述。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterValidationError {
    /// 参数在本模型中的点号路径。
    pub field: String,
    /// 稳定错误码，供 UI 和 IPC 使用，不应依赖 message 文案。
    pub code: String,
    /// 参数单位；枚举、ID 和关系错误使用 `"无"` 或对应比较单位。
    pub unit: String,
    /// 可报告的实际数值；非有限浮点数不写入，避免错误对象无法 JSON 序列化。
    pub value: Option<f64>,
    /// 允许范围下界（如果该错误有下界）。
    pub min: Option<f64>,
    /// 允许范围上界（如果该错误有上界）。
    pub max: Option<f64>,
    /// 面向日志和调试的可读信息。
    pub message: String,
}

/// 音频研究参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioResearchParams {
    /// 自然真人模式，单位为枚举值。
    pub natural_voice_mode: NaturalVoiceMode,
    /// 随机变声周期，单位为毫秒；范围 500–60,000 ms。
    pub random_change_period_ms: u64,
    /// 音高微移，单位为半音；范围 -2–2 semitone。
    pub pitch_shift_semitones: f64,
    /// 频谱扰动幅度，单位为相对百分比；范围 0–10%。
    pub spectral_perturbation_percent: f64,
    /// 环境噪声混入比例，单位为百分比；范围 0–100%。
    pub environment_noise_percent: f64,
    /// 源环境底噪电平，单位为 dBFS；范围 -60–-20 dBFS。
    pub environment_noise_dbfs: f64,
    /// MFCC 相对偏移，单位为百分比；范围 -20–20%。
    pub mfcc_shift_percent: f64,
    /// 相位扰动幅度，单位为相对百分比；范围 -20–20%。
    pub phase_perturbation_percent: f64,
    /// 响度调整，单位为 dB；范围 -6–6 dB。
    pub loudness_adjustment_db: f64,
    /// 输入增益，单位为 dB；范围 -6–6 dB。
    pub input_gain_db: f64,
    /// 输出增益，单位为 dB；范围 -6–6 dB。
    pub output_gain_db: f64,
    /// 干湿比，单位为百分比；范围 0–100%。
    pub dry_wet_percent: f64,
    /// 轻混响湿声比例，单位为百分比；范围 0–20%。
    pub reverb_wet_percent: f64,
    /// MFCC 分析维度，单位为阶；范围 1–40 阶。
    pub mfcc_dimensions: u8,
    /// SNR 浮动，单位为 dB；范围 -6–6 dB。
    pub snr_variation_db: f64,
    /// 共振峰偏移，单位为相对百分比；范围 -5–5%。
    pub formant_shift_percent: f64,
    /// 颤音频率，单位为 Hz；范围 3–8 Hz。
    pub vibrato_frequency_hz: f64,
    /// 颤音深度，单位为百分比；范围 0–3%。
    pub vibrato_depth_percent: f64,
    /// 频谱盲区宽度，单位为百分比；范围 0–5%。
    pub spectrum_blind_spot_percent: f64,
    /// 目标信噪比，单位为 dB；None 表示不设目标，由源素材/处理器读取。
    pub snr_target_db: Option<f64>,
    /// 当前共振峰测量值，单位为 Hz；None 表示尚未测量，范围 20–10000 Hz。
    pub current_formant_hz: Option<f64>,
    /// 滤波器 Q 值，无量纲；范围 0.3–10。
    pub filter_q: f64,
    /// 目标采样率，单位为 Hz；None 表示跟随源素材，显式值限制为 44100 或 48000。
    pub sample_rate_hz: Option<u32>,
    /// 输出音频码率，单位为 kbps；范围 64–320 kbps。
    pub output_bitrate_kbps: u16,
    /// 音色库资源 ID，无物理单位；None 表示跟随源音色。
    pub voice_library_id: Option<String>,
}

impl Default for AudioResearchParams {
    fn default() -> Self {
        Self {
            natural_voice_mode: NaturalVoiceMode::Original,
            random_change_period_ms: 5_000,
            pitch_shift_semitones: 0.0,
            spectral_perturbation_percent: 0.0,
            environment_noise_percent: 0.0,
            environment_noise_dbfs: -40.0,
            mfcc_shift_percent: 0.0,
            phase_perturbation_percent: 0.0,
            loudness_adjustment_db: 0.0,
            input_gain_db: 0.0,
            output_gain_db: 0.0,
            dry_wet_percent: 0.0,
            reverb_wet_percent: 0.0,
            mfcc_dimensions: 13,
            snr_variation_db: 0.0,
            formant_shift_percent: 0.0,
            vibrato_frequency_hz: 5.0,
            vibrato_depth_percent: 0.0,
            spectrum_blind_spot_percent: 0.0,
            snr_target_db: None,
            current_formant_hz: None,
            filter_q: 1.0,
            sample_rate_hz: None,
            output_bitrate_kbps: 192,
            voice_library_id: None,
        }
    }
}

/// 视频研究参数；所有空间值针对输出画面，单位在字段名中明确。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoResearchParams {
    /// 亮度偏移，单位为百分比；范围 -100–100%。
    pub brightness_percent: f64,
    /// 饱和度，单位为百分比；范围 0–200%。
    pub saturation_percent: f64,
    /// 模糊半径，单位为像素；范围 0–8 px。
    pub blur_radius_px: f64,
    /// 对比度，单位为百分比；范围 0–200%。
    pub contrast_percent: f64,
    /// 色相旋转，单位为度；范围 -180–180°。
    pub hue_rotation_degrees: f64,
    /// 锐化强度，单位为百分比；范围 0–100%。
    pub sharpen_percent: f64,
    /// 噪点强度，单位为百分比；范围 0–8%。
    pub noise_percent: f64,
    /// 细节增强强度，单位为百分比；范围 0–50%。
    pub detail_enhancement_percent: f64,
    /// 裁剪边缘平滑度，单位为归一化值；范围 0–1。
    pub crop_edge_smoothing: f64,
    /// 帧率微扰幅度，单位为百分比；范围 0–2%。
    pub frame_rate_jitter_percent: f64,
    /// 帧率微扰频率，单位为 Hz；范围 0.01–2 Hz。
    pub frame_rate_perturbation_frequency_hz: f64,
    /// 帧率微扰幅度，单位为 fps；范围 0–2 fps。
    pub frame_rate_perturbation_amplitude_fps: f64,
    /// 像素级缩放比例，单位为百分比；范围 95–105%。
    pub pixel_scale_percent: f64,
    /// 像素级扰动幅度，单位为像素；范围 0–2 px。
    pub pixel_jitter_px: f64,
    /// 动态裁剪幅度，单位为每边百分比；范围 0–4%。
    pub dynamic_crop_percent: f64,
    /// 帧内微扰幅度，单位为百分比；范围 0–2%。
    pub frame_inner_perturbation_percent: f64,
    /// 帧间微扰概率/幅度，单位为百分比；范围 0–20%。
    pub frame_inter_perturbation_percent: f64,
    /// 水平空间偏移，单位为像素；范围 -4–4 px。
    pub space_x_offset_px: f64,
    /// 垂直空间偏移，单位为像素；范围 -4–4 px。
    pub space_y_offset_px: f64,
    /// 色域转换强度，单位为百分比；范围 0–100%。
    pub color_space_conversion_strength_percent: f64,
}

impl Default for VideoResearchParams {
    fn default() -> Self {
        Self {
            brightness_percent: 0.0,
            saturation_percent: 100.0,
            blur_radius_px: 0.0,
            contrast_percent: 100.0,
            hue_rotation_degrees: 0.0,
            sharpen_percent: 0.0,
            noise_percent: 0.0,
            detail_enhancement_percent: 0.0,
            crop_edge_smoothing: 0.5,
            frame_rate_jitter_percent: 0.0,
            frame_rate_perturbation_frequency_hz: 0.1,
            frame_rate_perturbation_amplitude_fps: 0.0,
            pixel_scale_percent: 100.0,
            pixel_jitter_px: 0.0,
            dynamic_crop_percent: 0.0,
            frame_inner_perturbation_percent: 0.0,
            frame_inter_perturbation_percent: 0.0,
            space_x_offset_px: 0.0,
            space_y_offset_px: 0.0,
            color_space_conversion_strength_percent: 0.0,
        }
    }
}

/// 视频视觉调制、研究挂件和切片实验参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResearchExperimentParams {
    /// 固定视觉频段到权重倍数的映射，键单位为 Hz，值单位为倍数；每个固定频段都必须存在。
    pub band_weights: BTreeMap<u32, f64>,
    /// 目标视觉调制频率，单位为 Hz；None 表示关闭。
    pub target_frequency_hz: Option<f64>,
    /// 核心视觉调制频率，单位为 Hz；None 表示跟随目标频率。
    pub core_frequency_hz: Option<f64>,
    /// 波频强度，单位为归一化值；范围 0–1。
    pub wave_intensity: f64,
    /// 波频电平，单位为归一化值；范围 0–1。
    pub wave_level: f64,
    /// 波频颗粒数量，单位为个；范围 1–100。
    pub wave_grain_count: u32,
    /// 动态均衡阈值，单位为归一化刻度；范围 0–20。
    pub dynamic_eq_threshold: f64,
    /// 通道偏移，单位为百分比；范围 -10–10%。
    pub channel_offset_percent: f64,
    /// 视觉调制空间维度，单位为维；范围 1–3 维。
    pub space_dimension: u8,
    /// 视觉调制水平偏移，单位为像素；范围 -10–10 px。
    pub frequency_space_x_offset_px: f64,
    /// 视觉调制垂直偏移，单位为像素；范围 -10–10 px。
    pub frequency_space_y_offset_px: f64,
    /// 视觉调制帧扰动概率，单位为百分比；范围 0–20%。
    pub frame_perturbation_probability_percent: f64,
    /// 随机图形透明度，单位为百分比；范围 0–50%。
    pub random_graphic_opacity_percent: f64,
    /// 随机图形大小，单位为像素；范围 1–64 px。
    pub random_graphic_size_px: f64,
    /// 抽象人脸数量，单位为个；范围 0–10 个。
    pub abstract_face_count: u8,
    /// 抽象人脸大小，单位为画面宽度百分比；范围 1–10%。
    pub abstract_face_size_percent: f64,
    /// 抽象人脸透明度，单位为百分比；范围 0–30%。
    pub abstract_face_opacity_percent: f64,
    /// 挂件画面偏移，单位为像素；范围 -10–10 px。
    pub overlay_offset_px: f64,
    /// 一次切片/挂件效果持续时间，单位为毫秒；范围 500–10,000 ms。
    pub slice_length_ms: u64,
    /// 可选源片段最小长度，单位为毫秒；范围 1,000–60,000 ms。
    pub slice_min_length_ms: u64,
    /// 切片触发间隔，单位为毫秒；范围 5,000–120,000 ms，且不得短于切片长度。
    pub slice_trigger_interval_ms: u64,
}

impl Default for ResearchExperimentParams {
    fn default() -> Self {
        let band_weights = VISUAL_BAND_FREQUENCIES_HZ
            .iter()
            .copied()
            .map(|frequency_hz| (frequency_hz, 1.0))
            .collect();

        Self {
            band_weights,
            target_frequency_hz: None,
            core_frequency_hz: None,
            wave_intensity: 0.0,
            wave_level: 0.0,
            wave_grain_count: 20,
            dynamic_eq_threshold: 10.0,
            channel_offset_percent: 0.0,
            space_dimension: 2,
            frequency_space_x_offset_px: 0.0,
            frequency_space_y_offset_px: 0.0,
            frame_perturbation_probability_percent: 0.0,
            random_graphic_opacity_percent: 0.0,
            random_graphic_size_px: 4.0,
            abstract_face_count: 0,
            abstract_face_size_percent: 2.0,
            abstract_face_opacity_percent: 0.0,
            overlay_offset_px: 0.0,
            slice_length_ms: 5_000,
            slice_min_length_ms: 10_000,
            slice_trigger_interval_ms: 15_000,
        }
    }
}

/// 本地研究参数根模型。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LocalResearchParams {
    /// 音频研究参数。
    pub audio: AudioResearchParams,
    /// 视频研究参数。
    pub video: VideoResearchParams,
    /// 视觉频段、挂件和切片研究参数。
    pub research: ResearchExperimentParams,
}

impl LocalResearchParams {
    /// 校验完整参数树；成功只表示数据满足契约，不表示媒体处理已实现。
    pub fn validate(&self) -> Result<(), Vec<ParameterValidationError>> {
        let mut errors = Vec::new();
        self.audio.validate_into(&mut errors);
        self.video.validate_into(&mut errors);
        self.research.validate_into(&mut errors);
        finish_validation(errors)
    }
}

impl AudioResearchParams {
    /// 校验音频参数范围和音色库 ID 基本格式。
    pub fn validate(&self) -> Result<(), Vec<ParameterValidationError>> {
        let mut errors = Vec::new();
        self.validate_into(&mut errors);
        finish_validation(errors)
    }

    fn validate_into(&self, errors: &mut Vec<ParameterValidationError>) {
        validate_u64_range(
            errors,
            "audio.random_change_period_ms",
            "ms",
            self.random_change_period_ms,
            500,
            60_000,
        );
        validate_range(
            errors,
            "audio.pitch_shift_semitones",
            "semitone",
            self.pitch_shift_semitones,
            -2.0,
            2.0,
        );
        validate_range(
            errors,
            "audio.spectral_perturbation_percent",
            "%",
            self.spectral_perturbation_percent,
            0.0,
            10.0,
        );
        validate_range(
            errors,
            "audio.environment_noise_percent",
            "%",
            self.environment_noise_percent,
            0.0,
            100.0,
        );
        validate_range(
            errors,
            "audio.mfcc_shift_percent",
            "%",
            self.mfcc_shift_percent,
            -20.0,
            20.0,
        );
        validate_range(
            errors,
            "audio.phase_perturbation_percent",
            "%",
            self.phase_perturbation_percent,
            -20.0,
            20.0,
        );
        validate_range(
            errors,
            "audio.environment_noise_dbfs",
            "dBFS",
            self.environment_noise_dbfs,
            -60.0,
            -20.0,
        );
        validate_range(
            errors,
            "audio.loudness_adjustment_db",
            "dB",
            self.loudness_adjustment_db,
            -6.0,
            6.0,
        );
        validate_range(
            errors,
            "audio.input_gain_db",
            "dB",
            self.input_gain_db,
            -6.0,
            6.0,
        );
        validate_range(
            errors,
            "audio.output_gain_db",
            "dB",
            self.output_gain_db,
            -6.0,
            6.0,
        );
        validate_range(
            errors,
            "audio.dry_wet_percent",
            "%",
            self.dry_wet_percent,
            0.0,
            100.0,
        );
        validate_range(
            errors,
            "audio.reverb_wet_percent",
            "%",
            self.reverb_wet_percent,
            0.0,
            20.0,
        );
        validate_u8_range(
            errors,
            "audio.mfcc_dimensions",
            "阶",
            self.mfcc_dimensions,
            1,
            40,
        );
        validate_range(
            errors,
            "audio.snr_variation_db",
            "dB",
            self.snr_variation_db,
            -6.0,
            6.0,
        );
        validate_range(
            errors,
            "audio.formant_shift_percent",
            "%",
            self.formant_shift_percent,
            -5.0,
            5.0,
        );
        validate_range(
            errors,
            "audio.vibrato_frequency_hz",
            "Hz",
            self.vibrato_frequency_hz,
            3.0,
            8.0,
        );
        validate_range(
            errors,
            "audio.vibrato_depth_percent",
            "%",
            self.vibrato_depth_percent,
            0.0,
            3.0,
        );
        validate_range(
            errors,
            "audio.spectrum_blind_spot_percent",
            "%",
            self.spectrum_blind_spot_percent,
            0.0,
            5.0,
        );
        validate_optional_range(
            errors,
            "audio.snr_target_db",
            "dB",
            self.snr_target_db,
            0.0,
            60.0,
        );
        validate_optional_range(
            errors,
            "audio.current_formant_hz",
            "Hz",
            self.current_formant_hz,
            20.0,
            10_000.0,
        );
        validate_range(errors, "audio.filter_q", "无量纲", self.filter_q, 0.3, 10.0);
        if self
            .sample_rate_hz
            .is_some_and(|value| !matches!(value, 44_100 | 48_000))
        {
            errors.push(ParameterValidationError {
                field: "audio.sample_rate_hz".to_owned(),
                code: "unsupported_value".to_owned(),
                unit: "Hz".to_owned(),
                value: self.sample_rate_hz.map(f64::from),
                min: Some(44_100.0),
                max: Some(48_000.0),
                message: "采样率只能跟随源素材、44100 Hz 或 48000 Hz".to_owned(),
            });
        }
        validate_u16_range(
            errors,
            "audio.output_bitrate_kbps",
            "kbps",
            self.output_bitrate_kbps,
            64,
            320,
        );
        if self
            .voice_library_id
            .as_deref()
            .is_some_and(|voice_library_id| {
                voice_library_id.is_empty() || voice_library_id.len() > 128
            })
        {
            errors.push(ParameterValidationError {
                field: "audio.voice_library_id".to_owned(),
                code: "invalid_identifier".to_owned(),
                unit: "ID".to_owned(),
                value: None,
                min: None,
                max: Some(128.0),
                message: "音色库 ID 不能为空且长度不能超过 128 个字节".to_owned(),
            });
        }
    }
}

impl VideoResearchParams {
    /// 校验视频参数范围；不执行重新编码或滤镜处理。
    pub fn validate(&self) -> Result<(), Vec<ParameterValidationError>> {
        let mut errors = Vec::new();
        self.validate_into(&mut errors);
        finish_validation(errors)
    }

    fn validate_into(&self, errors: &mut Vec<ParameterValidationError>) {
        validate_range(
            errors,
            "video.brightness_percent",
            "%",
            self.brightness_percent,
            -100.0,
            100.0,
        );
        validate_range(
            errors,
            "video.saturation_percent",
            "%",
            self.saturation_percent,
            0.0,
            200.0,
        );
        validate_range(
            errors,
            "video.blur_radius_px",
            "px",
            self.blur_radius_px,
            0.0,
            8.0,
        );
        validate_range(
            errors,
            "video.contrast_percent",
            "%",
            self.contrast_percent,
            0.0,
            200.0,
        );
        validate_range(
            errors,
            "video.hue_rotation_degrees",
            "°",
            self.hue_rotation_degrees,
            -180.0,
            180.0,
        );
        validate_range(
            errors,
            "video.sharpen_percent",
            "%",
            self.sharpen_percent,
            0.0,
            100.0,
        );
        validate_range(
            errors,
            "video.noise_percent",
            "%",
            self.noise_percent,
            0.0,
            8.0,
        );
        validate_range(
            errors,
            "video.detail_enhancement_percent",
            "%",
            self.detail_enhancement_percent,
            0.0,
            50.0,
        );
        validate_range(
            errors,
            "video.crop_edge_smoothing",
            "归一化",
            self.crop_edge_smoothing,
            0.0,
            1.0,
        );
        validate_range(
            errors,
            "video.frame_rate_jitter_percent",
            "%",
            self.frame_rate_jitter_percent,
            0.0,
            2.0,
        );
        validate_range(
            errors,
            "video.frame_rate_perturbation_frequency_hz",
            "Hz",
            self.frame_rate_perturbation_frequency_hz,
            0.01,
            2.0,
        );
        validate_range(
            errors,
            "video.frame_rate_perturbation_amplitude_fps",
            "fps",
            self.frame_rate_perturbation_amplitude_fps,
            0.0,
            2.0,
        );
        validate_range(
            errors,
            "video.pixel_scale_percent",
            "%",
            self.pixel_scale_percent,
            95.0,
            105.0,
        );
        validate_range(
            errors,
            "video.pixel_jitter_px",
            "px",
            self.pixel_jitter_px,
            0.0,
            2.0,
        );
        validate_range(
            errors,
            "video.dynamic_crop_percent",
            "%/edge",
            self.dynamic_crop_percent,
            0.0,
            4.0,
        );
        validate_range(
            errors,
            "video.frame_inner_perturbation_percent",
            "%",
            self.frame_inner_perturbation_percent,
            0.0,
            2.0,
        );
        validate_range(
            errors,
            "video.frame_inter_perturbation_percent",
            "%",
            self.frame_inter_perturbation_percent,
            0.0,
            20.0,
        );
        validate_range(
            errors,
            "video.space_x_offset_px",
            "px",
            self.space_x_offset_px,
            -4.0,
            4.0,
        );
        validate_range(
            errors,
            "video.space_y_offset_px",
            "px",
            self.space_y_offset_px,
            -4.0,
            4.0,
        );
        validate_range(
            errors,
            "video.color_space_conversion_strength_percent",
            "%",
            self.color_space_conversion_strength_percent,
            0.0,
            100.0,
        );
    }
}

impl ResearchExperimentParams {
    /// 校验固定频段、视觉调制和挂件/切片参数。
    pub fn validate(&self) -> Result<(), Vec<ParameterValidationError>> {
        let mut errors = Vec::new();
        self.validate_into(&mut errors);
        finish_validation(errors)
    }

    fn validate_into(&self, errors: &mut Vec<ParameterValidationError>) {
        for frequency_hz in self.band_weights.keys().copied() {
            if !VISUAL_BAND_FREQUENCIES_HZ.contains(&frequency_hz) {
                errors.push(ParameterValidationError {
                    field: "research.band_weights".to_owned(),
                    code: "unknown_frequency_band".to_owned(),
                    unit: "Hz".to_owned(),
                    value: Some(frequency_hz as f64),
                    min: None,
                    max: None,
                    message: format!("不支持视觉频段 {frequency_hz} Hz"),
                });
            }
        }
        for frequency_hz in VISUAL_BAND_FREQUENCIES_HZ {
            match self.band_weights.get(&frequency_hz) {
                Some(weight) => validate_range(
                    errors,
                    &format!("research.band_weights.{frequency_hz}"),
                    "倍",
                    *weight,
                    0.5,
                    1.5,
                ),
                None => errors.push(ParameterValidationError {
                    field: "research.band_weights".to_owned(),
                    code: "missing_frequency_band".to_owned(),
                    unit: "Hz".to_owned(),
                    value: Some(frequency_hz as f64),
                    min: None,
                    max: None,
                    message: format!("缺少固定视觉频段 {frequency_hz} Hz"),
                }),
            }
        }
        validate_optional_range(
            errors,
            "research.target_frequency_hz",
            "Hz",
            self.target_frequency_hz,
            65.0,
            20_000.0,
        );
        validate_optional_range(
            errors,
            "research.core_frequency_hz",
            "Hz",
            self.core_frequency_hz,
            65.0,
            20_000.0,
        );
        if self.target_frequency_hz.is_none() && self.core_frequency_hz.is_some() {
            errors.push(ParameterValidationError {
                field: "research.core_frequency_hz".to_owned(),
                code: "invalid_relation".to_owned(),
                unit: "Hz".to_owned(),
                value: self.core_frequency_hz,
                min: None,
                max: None,
                message: "核心频率不能在目标频率关闭时单独启用".to_owned(),
            });
        }
        validate_range(
            errors,
            "research.wave_intensity",
            "归一化",
            self.wave_intensity,
            0.0,
            1.0,
        );
        validate_range(
            errors,
            "research.wave_level",
            "归一化",
            self.wave_level,
            0.0,
            1.0,
        );
        validate_u32_range(
            errors,
            "research.wave_grain_count",
            "个",
            self.wave_grain_count,
            1,
            100,
        );
        validate_range(
            errors,
            "research.dynamic_eq_threshold",
            "归一化刻度",
            self.dynamic_eq_threshold,
            0.0,
            20.0,
        );
        validate_range(
            errors,
            "research.channel_offset_percent",
            "%",
            self.channel_offset_percent,
            -10.0,
            10.0,
        );
        validate_u8_range(
            errors,
            "research.space_dimension",
            "维",
            self.space_dimension,
            1,
            3,
        );
        validate_range(
            errors,
            "research.frequency_space_x_offset_px",
            "px",
            self.frequency_space_x_offset_px,
            -10.0,
            10.0,
        );
        validate_range(
            errors,
            "research.frequency_space_y_offset_px",
            "px",
            self.frequency_space_y_offset_px,
            -10.0,
            10.0,
        );
        validate_range(
            errors,
            "research.frame_perturbation_probability_percent",
            "%",
            self.frame_perturbation_probability_percent,
            0.0,
            20.0,
        );
        validate_range(
            errors,
            "research.random_graphic_opacity_percent",
            "%",
            self.random_graphic_opacity_percent,
            0.0,
            50.0,
        );
        validate_range(
            errors,
            "research.random_graphic_size_px",
            "px",
            self.random_graphic_size_px,
            1.0,
            64.0,
        );
        validate_u8_range(
            errors,
            "research.abstract_face_count",
            "个",
            self.abstract_face_count,
            0,
            10,
        );
        validate_range(
            errors,
            "research.abstract_face_size_percent",
            "%/width",
            self.abstract_face_size_percent,
            1.0,
            10.0,
        );
        validate_range(
            errors,
            "research.abstract_face_opacity_percent",
            "%",
            self.abstract_face_opacity_percent,
            0.0,
            30.0,
        );
        validate_range(
            errors,
            "research.overlay_offset_px",
            "px",
            self.overlay_offset_px,
            -10.0,
            10.0,
        );
        validate_u64_range(
            errors,
            "research.slice_length_ms",
            "ms",
            self.slice_length_ms,
            500,
            10_000,
        );
        validate_u64_range(
            errors,
            "research.slice_min_length_ms",
            "ms",
            self.slice_min_length_ms,
            1_000,
            60_000,
        );
        validate_u64_range(
            errors,
            "research.slice_trigger_interval_ms",
            "ms",
            self.slice_trigger_interval_ms,
            5_000,
            120_000,
        );
        if self.slice_trigger_interval_ms < self.slice_length_ms {
            errors.push(ParameterValidationError {
                field: "research.slice_trigger_interval_ms".to_owned(),
                code: "invalid_relation".to_owned(),
                unit: "ms".to_owned(),
                value: Some(self.slice_trigger_interval_ms as f64),
                min: Some(self.slice_length_ms as f64),
                max: None,
                message: "切片触发间隔不能短于切片长度".to_owned(),
            });
        }
    }
}

fn finish_validation(
    errors: Vec<ParameterValidationError>,
) -> Result<(), Vec<ParameterValidationError>> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_range(
    errors: &mut Vec<ParameterValidationError>,
    field: &str,
    unit: &str,
    value: f64,
    min: f64,
    max: f64,
) {
    if !value.is_finite() || !(min..=max).contains(&value) {
        errors.push(ParameterValidationError {
            field: field.to_owned(),
            code: "out_of_range".to_owned(),
            unit: unit.to_owned(),
            value: value.is_finite().then_some(value),
            min: Some(min),
            max: Some(max),
            message: format!("参数必须位于 {min}–{max} {unit} 范围内"),
        });
    }
}

fn validate_optional_range(
    errors: &mut Vec<ParameterValidationError>,
    field: &str,
    unit: &str,
    value: Option<f64>,
    min: f64,
    max: f64,
) {
    if let Some(value) = value {
        validate_range(errors, field, unit, value, min, max);
    }
}

fn validate_u64_range(
    errors: &mut Vec<ParameterValidationError>,
    field: &str,
    unit: &str,
    value: u64,
    min: u64,
    max: u64,
) {
    validate_range(errors, field, unit, value as f64, min as f64, max as f64);
}

fn validate_u32_range(
    errors: &mut Vec<ParameterValidationError>,
    field: &str,
    unit: &str,
    value: u32,
    min: u32,
    max: u32,
) {
    validate_range(errors, field, unit, value as f64, min as f64, max as f64);
}

fn validate_u8_range(
    errors: &mut Vec<ParameterValidationError>,
    field: &str,
    unit: &str,
    value: u8,
    min: u8,
    max: u8,
) {
    validate_range(errors, field, unit, value as f64, min as f64, max as f64);
}

fn validate_u16_range(
    errors: &mut Vec<ParameterValidationError>,
    field: &str,
    unit: &str,
    value: u16,
    min: u16,
    max: u16,
) {
    if !(min..=max).contains(&value) {
        errors.push(ParameterValidationError {
            field: field.to_owned(),
            code: "out_of_range".to_owned(),
            unit: unit.to_owned(),
            value: Some(f64::from(value)),
            min: Some(f64::from(min)),
            max: Some(f64::from(max)),
            message: format!("参数必须位于 {min}–{max} {unit} 范围内"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioResearchParams, LocalResearchParams, VideoResearchParams, VISUAL_BAND_FREQUENCIES_HZ,
    };

    #[test]
    fn default_parameters_are_serializable_and_valid() {
        let params = LocalResearchParams::default();

        assert!(params.validate().is_ok());

        let encoded = serde_json::to_string(&params).expect("default params should serialize");
        let decoded: LocalResearchParams =
            serde_json::from_str(&encoded).expect("serialized params should deserialize");
        assert_eq!(decoded, params);
    }

    #[test]
    fn normal_values_are_accepted() {
        let params = LocalResearchParams {
            audio: AudioResearchParams {
                random_change_period_ms: 30_000,
                pitch_shift_semitones: -1.5,
                spectral_perturbation_percent: 4.0,
                environment_noise_percent: 2.0,
                mfcc_shift_percent: 10.0,
                phase_perturbation_percent: -5.0,
                snr_target_db: Some(35.0),
                current_formant_hz: Some(120.0),
                filter_q: 0.8,
                voice_library_id: Some("source".to_owned()),
                ..AudioResearchParams::default()
            },
            video: VideoResearchParams {
                brightness_percent: 5.0,
                saturation_percent: 105.0,
                blur_radius_px: 1.0,
                contrast_percent: 98.0,
                hue_rotation_degrees: -3.0,
                sharpen_percent: 5.0,
                noise_percent: 1.0,
                detail_enhancement_percent: 8.0,
                frame_rate_jitter_percent: 0.5,
                pixel_scale_percent: 100.2,
                pixel_jitter_px: 0.5,
                dynamic_crop_percent: 1.0,
                frame_inner_perturbation_percent: 0.5,
                frame_inter_perturbation_percent: 4.0,
                space_x_offset_px: 1.0,
                space_y_offset_px: -1.0,
                color_space_conversion_strength_percent: 10.0,
                ..VideoResearchParams::default()
            },
            ..LocalResearchParams::default()
        };

        assert!(params.validate().is_ok());
    }

    #[test]
    fn fixed_visual_band_frequency_set_is_stable() {
        assert_eq!(
            VISUAL_BAND_FREQUENCIES_HZ,
            [65, 92, 131, 188, 267, 381, 544, 777, 1110, 1585, 2263, 20_000]
        );

        let params = LocalResearchParams::default();
        let frequencies: Vec<_> = params.research.band_weights.keys().copied().collect();
        assert_eq!(frequencies, VISUAL_BAND_FREQUENCIES_HZ);
    }

    #[test]
    fn out_of_range_values_return_structured_errors() {
        let mut params = LocalResearchParams::default();
        params.audio.pitch_shift_semitones = 3.0;
        params.video.brightness_percent = -101.0;
        params.research.band_weights.insert(64, 1.0);

        let errors = params.validate().expect_err("invalid params should fail");

        assert!(errors.iter().any(|error| {
            error.field == "audio.pitch_shift_semitones"
                && error.code == "out_of_range"
                && error.unit == "semitone"
        }));
        assert!(errors.iter().any(|error| {
            error.field == "video.brightness_percent"
                && error.code == "out_of_range"
                && error.unit == "%"
        }));
        assert!(errors.iter().any(|error| {
            error.field == "research.band_weights"
                && error.code == "unknown_frequency_band"
                && error.unit == "Hz"
        }));
    }

    #[test]
    fn slice_trigger_interval_cannot_be_shorter_than_slice() {
        let mut params = LocalResearchParams::default();
        params.research.slice_length_ms = 10_000;
        params.research.slice_trigger_interval_ms = 5_000;

        let errors = params
            .validate()
            .expect_err("invalid slice timing should fail");

        assert!(errors.iter().any(|error| {
            error.field == "research.slice_trigger_interval_ms"
                && error.code == "invalid_relation"
                && error.unit == "ms"
        }));
    }
}
