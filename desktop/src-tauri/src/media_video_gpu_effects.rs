use crate::media_effect_params::{AdvancedEffectParams, VideoEffectParams};
use crate::media_video_effects::{
    atomic_media_video_ui_applied_fields, build_atomic_media_video_effect_plan,
};
use crate::media_video_frame_scheduler::{VideoFrameSchedule, MAX_RANDOM_GRAPHIC_SEED};
use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;

/// 随包静态 shader。mpv 会话只加载一次，周期更新不得改写这份源码。
pub const GPU83_SHADER_SOURCE: &str = include_str!("../resources/shaders/gpu83.hook");

pub const GPU83_SHADER_RESOURCE_PATH: &str = "shaders/gpu83.hook";
pub const GPU83_SHADER_OPTIONS_PROPERTY: &str = "glsl-shader-opts";

pub const GPU83_PARAMETER_COUNT: usize = 83;
const REQUIRES_HISTORY_TEXTURE: &str = "requires_history_or_secondary_texture";
// mpv 的 renderer color-map 状态不能与 glsl-shader-opts 组成同帧事务；
// 未取得源/目标色彩元数据时也不得用固定 RGB 矩阵冒充感知色域映射。
const ALGORITHM_NOT_VERIFIED: &str = "algorithm_semantics_not_verified";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gpu83ExecutionClass {
    PixelShader,
    CompositeShader,
    FrameScheduling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gpu83ParameterCapability {
    ShaderParameter,
    ScheduledParameter,
    Unavailable(&'static str),
}

#[derive(Clone, Copy)]
pub struct Gpu83ParameterMapping {
    pub field_path: &'static str,
    pub shader_option: &'static str,
    pub execution_class: Gpu83ExecutionClass,
    pub capability: Gpu83ParameterCapability,
    required: bool,
    read: fn(&VideoEffectParams, &AdvancedEffectParams) -> Option<f64>,
}

impl std::fmt::Debug for Gpu83ParameterMapping {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Gpu83ParameterMapping")
            .field("field_path", &self.field_path)
            .field("shader_option", &self.shader_option)
            .field("execution_class", &self.execution_class)
            .field("capability", &self.capability)
            .finish_non_exhaustive()
    }
}

macro_rules! video_value {
    ($path:literal, $option:literal, $field:ident, $class:ident, $capability:expr) => {
        Gpu83ParameterMapping {
            field_path: $path,
            shader_option: $option,
            execution_class: Gpu83ExecutionClass::$class,
            capability: $capability,
            required: true,
            read: |video, _| Some(video.$field),
        }
    };
}

macro_rules! video_flag {
    ($path:literal, $option:literal, $field:ident, $class:ident, $capability:expr) => {
        Gpu83ParameterMapping {
            field_path: $path,
            shader_option: $option,
            execution_class: Gpu83ExecutionClass::$class,
            capability: $capability,
            required: true,
            read: |video, _| Some(if video.$field { 1.0 } else { 0.0 }),
        }
    };
}

macro_rules! advanced_value {
    ($path:literal, $option:literal, $field:ident, $class:ident, $capability:expr) => {
        Gpu83ParameterMapping {
            field_path: $path,
            shader_option: $option,
            execution_class: Gpu83ExecutionClass::$class,
            capability: $capability,
            required: true,
            read: |_, advanced| Some(advanced.$field as f64),
        }
    };
}

macro_rules! advanced_flag {
    ($path:literal, $option:literal, $field:ident, $class:ident, $capability:expr) => {
        Gpu83ParameterMapping {
            field_path: $path,
            shader_option: $option,
            execution_class: Gpu83ExecutionClass::$class,
            capability: $capability,
            required: true,
            read: |_, advanced| Some(if advanced.$field { 1.0 } else { 0.0 }),
        }
    };
}

macro_rules! advanced_optional {
    ($path:literal, $option:literal, $field:ident, $class:ident, $capability:expr) => {
        Gpu83ParameterMapping {
            field_path: $path,
            shader_option: $option,
            execution_class: Gpu83ExecutionClass::$class,
            capability: $capability,
            required: false,
            read: |_, advanced| advanced.$field,
        }
    };
}

macro_rules! visual_band {
    ($frequency:literal, $path:literal, $option:literal) => {
        Gpu83ParameterMapping {
            field_path: $path,
            shader_option: $option,
            execution_class: Gpu83ExecutionClass::PixelShader,
            capability: AVAILABLE,
            required: true,
            read: |_, advanced| advanced.band_weights.get(&$frequency).copied(),
        }
    };
}

const AVAILABLE: Gpu83ParameterCapability = Gpu83ParameterCapability::ShaderParameter;
const UNVERIFIED: Gpu83ParameterCapability =
    Gpu83ParameterCapability::Unavailable(ALGORITHM_NOT_VERIFIED);
const SCHEDULER: Gpu83ParameterCapability = Gpu83ParameterCapability::ScheduledParameter;
const HISTORY: Gpu83ParameterCapability =
    Gpu83ParameterCapability::Unavailable(REQUIRES_HISTORY_TEXTURE);

/// 83 项的唯一执行映射。水平/垂直翻转不在此表；不可真实表达的项显式不可用。
pub static GPU83_PARAMETER_MAPPINGS: [Gpu83ParameterMapping; GPU83_PARAMETER_COUNT] = [
    video_value!(
        "video.brightness_percent",
        "al_brightness_percent",
        brightness_percent,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.saturation_percent",
        "al_saturation_percent",
        saturation_percent,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.blur_radius_px",
        "al_blur_radius_px",
        blur_radius_px,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.contrast_percent",
        "al_contrast_percent",
        contrast_percent,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.hue_rotation_degrees",
        "al_hue_degrees",
        hue_rotation_degrees,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.sharpen_percent",
        "al_sharpen_percent",
        sharpen_percent,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.noise_percent",
        "al_noise_percent",
        noise_percent,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.detail_enhancement_percent",
        "al_detail_percent",
        detail_enhancement_percent,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.crop_edge_smoothing",
        "al_crop_edge_smoothing",
        crop_edge_smoothing,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.frame_rate_jitter_percent",
        "al_frame_rate_jitter_percent",
        frame_rate_jitter_percent,
        FrameScheduling,
        SCHEDULER
    ),
    video_value!(
        "video.frame_rate_perturbation_frequency_hz",
        "al_frame_rate_frequency_hz",
        frame_rate_perturbation_frequency_hz,
        FrameScheduling,
        SCHEDULER
    ),
    video_value!(
        "video.frame_rate_perturbation_amplitude_fps",
        "al_frame_rate_amplitude_fps",
        frame_rate_perturbation_amplitude_fps,
        FrameScheduling,
        SCHEDULER
    ),
    video_value!(
        "video.pixel_scale_percent",
        "al_pixel_scale_percent",
        pixel_scale_percent,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.pixel_jitter_px",
        "al_pixel_jitter_px",
        pixel_jitter_px,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.dynamic_crop_percent",
        "al_dynamic_crop_percent",
        dynamic_crop_percent,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.frame_inner_perturbation_percent",
        "al_frame_inner_percent",
        frame_inner_perturbation_percent,
        FrameScheduling,
        SCHEDULER
    ),
    video_value!(
        "video.frame_inter_perturbation_percent",
        "al_frame_inter_percent",
        frame_inter_perturbation_percent,
        FrameScheduling,
        SCHEDULER
    ),
    video_value!(
        "video.space_x_offset_px",
        "al_space_x_px",
        space_x_offset_px,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.space_y_offset_px",
        "al_space_y_px",
        space_y_offset_px,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.color_space_conversion_strength_percent",
        "al_color_space_strength_percent",
        color_space_conversion_strength_percent,
        PixelShader,
        UNVERIFIED
    ),
    video_flag!(
        "video.color_space_conversion_enabled",
        "al_color_space_enabled",
        color_space_conversion_enabled,
        PixelShader,
        UNVERIFIED
    ),
    video_value!(
        "video.rotation_degrees",
        "al_rotation_degrees",
        rotation_degrees,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.vignette_percent",
        "al_vignette_percent",
        vignette_percent,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.highlights_percent",
        "al_highlights_percent",
        highlights_percent,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.shadows_percent",
        "al_shadows_percent",
        shadows_percent,
        PixelShader,
        AVAILABLE
    ),
    video_flag!(
        "video.red_channel_lock_enabled",
        "al_red_lock_enabled",
        red_channel_lock_enabled,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.edge_softness_percent",
        "al_edge_softness_percent",
        edge_softness_percent,
        PixelShader,
        AVAILABLE
    ),
    video_flag!(
        "video.image_repair_enabled",
        "al_image_repair_enabled",
        image_repair_enabled,
        PixelShader,
        AVAILABLE
    ),
    video_value!(
        "video.image_repair_strength_percent",
        "al_image_repair_strength_percent",
        image_repair_strength_percent,
        PixelShader,
        AVAILABLE
    ),
    video_flag!(
        "video.frame_rate_lock_enabled",
        "al_frame_rate_lock_enabled",
        frame_rate_lock_enabled,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_optional!(
        "advanced.target_frequency_hz",
        "al_target_frequency_hz",
        target_frequency_hz,
        PixelShader,
        AVAILABLE
    ),
    advanced_optional!(
        "advanced.core_frequency_hz",
        "al_core_frequency_hz",
        core_frequency_hz,
        PixelShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.wave_intensity",
        "al_wave_intensity",
        wave_intensity,
        PixelShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.wave_level",
        "al_wave_level",
        wave_level,
        PixelShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.wave_grain_count",
        "al_wave_grain_count",
        wave_grain_count,
        PixelShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.dynamic_eq_threshold",
        "al_dynamic_eq_threshold",
        dynamic_eq_threshold,
        PixelShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.channel_offset_percent",
        "al_channel_offset_percent",
        channel_offset_percent,
        PixelShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.space_dimension",
        "al_space_dimension",
        space_dimension,
        PixelShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.frequency_space_x_offset_px",
        "al_frequency_space_x_px",
        frequency_space_x_offset_px,
        PixelShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.frequency_space_y_offset_px",
        "al_frequency_space_y_px",
        frequency_space_y_offset_px,
        PixelShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.frame_perturbation_probability_percent",
        "al_frame_probability_percent",
        frame_perturbation_probability_percent,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_value!(
        "advanced.random_graphic_opacity_percent",
        "al_random_graphic_opacity_percent",
        random_graphic_opacity_percent,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.random_graphic_size_px",
        "al_random_graphic_size_px",
        random_graphic_size_px,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.abstract_face_count",
        "al_abstract_face_count",
        abstract_face_count,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.abstract_face_size_percent",
        "al_abstract_face_size_percent",
        abstract_face_size_percent,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.abstract_face_opacity_percent",
        "al_abstract_face_opacity_percent",
        abstract_face_opacity_percent,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.overlay_offset_px",
        "al_overlay_offset_px",
        overlay_offset_px,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.slice_length_ms",
        "al_slice_length_ms",
        slice_length_ms,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_value!(
        "advanced.slice_min_length_ms",
        "al_slice_min_length_ms",
        slice_min_length_ms,
        FrameScheduling,
        HISTORY
    ),
    advanced_value!(
        "advanced.slice_trigger_interval_ms",
        "al_slice_interval_ms",
        slice_trigger_interval_ms,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_flag!(
        "advanced.random_graphic_enabled",
        "al_random_graphic_enabled",
        random_graphic_enabled,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.random_graphic_count",
        "al_random_graphic_count",
        random_graphic_count,
        CompositeShader,
        AVAILABLE
    ),
    advanced_flag!(
        "advanced.picture_in_picture_enabled",
        "al_pip_enabled",
        picture_in_picture_enabled,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.picture_in_picture_scale_percent",
        "al_pip_scale_percent",
        picture_in_picture_scale_percent,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.picture_in_picture_opacity_percent",
        "al_pip_opacity_percent",
        picture_in_picture_opacity_percent,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.picture_in_picture_rotation_degrees",
        "al_pip_rotation_degrees",
        picture_in_picture_rotation_degrees,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.picture_in_picture_pixel_jitter_px",
        "al_pip_jitter_px",
        picture_in_picture_pixel_jitter_px,
        CompositeShader,
        SCHEDULER
    ),
    advanced_flag!(
        "advanced.picture_in_picture_timeline_locked",
        "al_pip_timeline_locked",
        picture_in_picture_timeline_locked,
        FrameScheduling,
        HISTORY
    ),
    advanced_flag!(
        "advanced.local_blur_enabled",
        "al_local_blur_enabled",
        local_blur_enabled,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.local_blur_region_percent",
        "al_local_blur_region_percent",
        local_blur_region_percent,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.local_blur_radius_px",
        "al_local_blur_radius_px",
        local_blur_radius_px,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.local_blur_interval_ms",
        "al_local_blur_interval_ms",
        local_blur_interval_ms,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_flag!(
        "advanced.edge_fill_enabled",
        "al_edge_fill_enabled",
        edge_fill_enabled,
        CompositeShader,
        AVAILABLE
    ),
    advanced_value!(
        "advanced.edge_feather_percent",
        "al_edge_feather_percent",
        edge_feather_percent,
        CompositeShader,
        AVAILABLE
    ),
    advanced_flag!(
        "advanced.transform_smoothing_enabled",
        "al_transform_smoothing_enabled",
        transform_smoothing_enabled,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_value!(
        "advanced.transform_smoothing_duration_ms",
        "al_transform_smoothing_ms",
        transform_smoothing_duration_ms,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_flag!(
        "advanced.highlight_perturbation_enabled",
        "al_highlight_perturbation_enabled",
        highlight_perturbation_enabled,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_value!(
        "advanced.highlight_perturbation_interval_ms",
        "al_highlight_interval_ms",
        highlight_perturbation_interval_ms,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_flag!(
        "advanced.asynchronous_rotation_enabled",
        "al_async_rotation_enabled",
        asynchronous_rotation_enabled,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_value!(
        "advanced.asynchronous_rotation_min_degrees",
        "al_async_rotation_min_degrees",
        asynchronous_rotation_min_degrees,
        FrameScheduling,
        SCHEDULER
    ),
    advanced_value!(
        "advanced.asynchronous_rotation_max_degrees",
        "al_async_rotation_max_degrees",
        asynchronous_rotation_max_degrees,
        FrameScheduling,
        SCHEDULER
    ),
    visual_band!(65, "advanced.band_weights.65", "al_band_65"),
    visual_band!(92, "advanced.band_weights.92", "al_band_92"),
    visual_band!(131, "advanced.band_weights.131", "al_band_131"),
    visual_band!(188, "advanced.band_weights.188", "al_band_188"),
    visual_band!(267, "advanced.band_weights.267", "al_band_267"),
    visual_band!(381, "advanced.band_weights.381", "al_band_381"),
    visual_band!(544, "advanced.band_weights.544", "al_band_544"),
    visual_band!(777, "advanced.band_weights.777", "al_band_777"),
    visual_band!(1_110, "advanced.band_weights.1110", "al_band_1110"),
    visual_band!(1_585, "advanced.band_weights.1585", "al_band_1585"),
    visual_band!(2_263, "advanced.band_weights.2263", "al_band_2263"),
    visual_band!(20_000, "advanced.band_weights.20000", "al_band_20000"),
];

pub const GPU83_FIELD_PATHS: [&str; GPU83_PARAMETER_COUNT] = [
    "video.brightness_percent",
    "video.saturation_percent",
    "video.blur_radius_px",
    "video.contrast_percent",
    "video.hue_rotation_degrees",
    "video.sharpen_percent",
    "video.noise_percent",
    "video.detail_enhancement_percent",
    "video.crop_edge_smoothing",
    "video.frame_rate_jitter_percent",
    "video.frame_rate_perturbation_frequency_hz",
    "video.frame_rate_perturbation_amplitude_fps",
    "video.pixel_scale_percent",
    "video.pixel_jitter_px",
    "video.dynamic_crop_percent",
    "video.frame_inner_perturbation_percent",
    "video.frame_inter_perturbation_percent",
    "video.space_x_offset_px",
    "video.space_y_offset_px",
    "video.color_space_conversion_strength_percent",
    "video.color_space_conversion_enabled",
    "video.rotation_degrees",
    "video.vignette_percent",
    "video.highlights_percent",
    "video.shadows_percent",
    "video.red_channel_lock_enabled",
    "video.edge_softness_percent",
    "video.image_repair_enabled",
    "video.image_repair_strength_percent",
    "video.frame_rate_lock_enabled",
    "advanced.target_frequency_hz",
    "advanced.core_frequency_hz",
    "advanced.wave_intensity",
    "advanced.wave_level",
    "advanced.wave_grain_count",
    "advanced.dynamic_eq_threshold",
    "advanced.channel_offset_percent",
    "advanced.space_dimension",
    "advanced.frequency_space_x_offset_px",
    "advanced.frequency_space_y_offset_px",
    "advanced.frame_perturbation_probability_percent",
    "advanced.random_graphic_opacity_percent",
    "advanced.random_graphic_size_px",
    "advanced.abstract_face_count",
    "advanced.abstract_face_size_percent",
    "advanced.abstract_face_opacity_percent",
    "advanced.overlay_offset_px",
    "advanced.slice_length_ms",
    "advanced.slice_min_length_ms",
    "advanced.slice_trigger_interval_ms",
    "advanced.random_graphic_enabled",
    "advanced.random_graphic_count",
    "advanced.picture_in_picture_enabled",
    "advanced.picture_in_picture_scale_percent",
    "advanced.picture_in_picture_opacity_percent",
    "advanced.picture_in_picture_rotation_degrees",
    "advanced.picture_in_picture_pixel_jitter_px",
    "advanced.picture_in_picture_timeline_locked",
    "advanced.local_blur_enabled",
    "advanced.local_blur_region_percent",
    "advanced.local_blur_radius_px",
    "advanced.local_blur_interval_ms",
    "advanced.edge_fill_enabled",
    "advanced.edge_feather_percent",
    "advanced.transform_smoothing_enabled",
    "advanced.transform_smoothing_duration_ms",
    "advanced.highlight_perturbation_enabled",
    "advanced.highlight_perturbation_interval_ms",
    "advanced.asynchronous_rotation_enabled",
    "advanced.asynchronous_rotation_min_degrees",
    "advanced.asynchronous_rotation_max_degrees",
    "advanced.band_weights.65",
    "advanced.band_weights.92",
    "advanced.band_weights.131",
    "advanced.band_weights.188",
    "advanced.band_weights.267",
    "advanced.band_weights.381",
    "advanced.band_weights.544",
    "advanced.band_weights.777",
    "advanced.band_weights.1110",
    "advanced.band_weights.1585",
    "advanced.band_weights.2263",
    "advanced.band_weights.20000",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Gpu83SnapshotEntry {
    pub field_path: &'static str,
    pub value: Option<f64>,
    pub capability: Gpu83ParameterCapability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MpvShaderOptionsUpdate {
    pub property: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gpu83ShaderSnapshot {
    pub entries: Vec<Gpu83SnapshotEntry>,
    pub unavailable_fields: Vec<&'static str>,
    shader_options: String,
}

impl Gpu83ShaderSnapshot {
    pub fn is_fully_available(&self) -> bool {
        self.unavailable_fields.is_empty()
    }

    /// 一轮计划只产生这一个属性更新；调用方不得再拆成逐字段 IPC。
    pub fn mpv_property_update(&self) -> MpvShaderOptionsUpdate {
        MpvShaderOptionsUpdate {
            property: GPU83_SHADER_OPTIONS_PROPERTY,
            value: self.shader_options.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu83SnapshotError {
    pub field: String,
    pub code: &'static str,
}

pub fn validate_gpu83_mapping_table(mappings: &[Gpu83ParameterMapping]) -> Result<(), Vec<String>> {
    let expected = GPU83_FIELD_PATHS.iter().copied().collect::<BTreeSet<_>>();
    let mut fields = BTreeSet::new();
    let mut options = HashSet::new();
    let mut errors = Vec::new();

    for mapping in mappings {
        if !fields.insert(mapping.field_path) {
            errors.push(format!("duplicate_field:{}", mapping.field_path));
        }
        if !options.insert(mapping.shader_option) {
            errors.push(format!("duplicate_option:{}", mapping.shader_option));
        }
        if !mapping
            .shader_option
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || mapping.shader_option.is_empty()
        {
            errors.push(format!("invalid_option:{}", mapping.shader_option));
        }
    }

    for missing in expected.difference(&fields) {
        errors.push(format!("missing_field:{missing}"));
    }
    for unexpected in fields.difference(&expected) {
        errors.push(format!("unexpected_field:{unexpected}"));
    }
    if fields.contains("video.horizontal_flip_enabled")
        || fields.contains("video.vertical_flip_enabled")
    {
        errors.push("flip_fields_must_not_enter_gpu83".to_owned());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn build_gpu83_shader_snapshot(
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
) -> Result<Gpu83ShaderSnapshot, Gpu83SnapshotError> {
    validate_gpu83_mapping_table(&GPU83_PARAMETER_MAPPINGS).map_err(|_| Gpu83SnapshotError {
        field: "gpu83.mapping".to_owned(),
        code: "invalid_mapping_table",
    })?;

    let mut entries = Vec::with_capacity(GPU83_PARAMETER_COUNT);
    let mut unavailable_fields = Vec::new();
    let mut options = Vec::new();
    for mapping in GPU83_PARAMETER_MAPPINGS {
        let value = (mapping.read)(video, advanced);
        if mapping.required && value.is_none() {
            return Err(Gpu83SnapshotError {
                field: mapping.field_path.to_owned(),
                code: "missing_value",
            });
        }
        if value.is_some_and(|value| !value.is_finite()) {
            return Err(Gpu83SnapshotError {
                field: mapping.field_path.to_owned(),
                code: "non_finite_value",
            });
        }
        match mapping.capability {
            Gpu83ParameterCapability::ShaderParameter => {
                let shader_value = value.unwrap_or(0.0);
                options.push(format!(
                    "{}={}",
                    mapping.shader_option,
                    format_shader_number(shader_value)
                ));
            }
            Gpu83ParameterCapability::ScheduledParameter => {}
            Gpu83ParameterCapability::Unavailable(_) => {
                unavailable_fields.push(mapping.field_path);
            }
        }
        entries.push(Gpu83SnapshotEntry {
            field_path: mapping.field_path,
            value,
            capability: mapping.capability,
        });
    }

    if let Err(errors) = video.validate() {
        let first = &errors[0];
        return Err(Gpu83SnapshotError {
            field: first.field.clone(),
            code: "invalid_parameter",
        });
    }
    if let Err(errors) = advanced.validate() {
        let first = &errors[0];
        return Err(Gpu83SnapshotError {
            field: first.field.clone(),
            code: "invalid_parameter",
        });
    }

    Ok(Gpu83ShaderSnapshot {
        entries,
        unavailable_fields,
        shader_options: options.join(","),
    })
}

/// 将一个视频周期的静态调度输入合并为一条完整 shader 属性快照。
/// 周期内的门控、抖动和旋转只由 shader 自动 `PTS` 推导，运行时不得逐帧重写属性。
pub fn build_gpu83_scheduled_shader_update(
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
    schedule: &VideoFrameSchedule,
) -> Result<MpvShaderOptionsUpdate, Gpu83SnapshotError> {
    let mut update = build_gpu83_shader_snapshot(video, advanced)?.mpv_property_update();
    let source_fps = schedule.target_fps / schedule.base_video_speed;
    for (field, value) in [("schedule.source_fps", source_fps)] {
        if !value.is_finite() {
            return Err(Gpu83SnapshotError {
                field: field.to_owned(),
                code: "non_finite_value",
            });
        }
    }
    if schedule.random_graphic_seed > MAX_RANDOM_GRAPHIC_SEED {
        return Err(Gpu83SnapshotError {
            field: "schedule.random_graphic_seed".to_owned(),
            code: "out_of_range",
        });
    }

    for (option, value) in [
        (
            "al_runtime_epoch_start_seconds",
            schedule.media_pts_ms as f64 / 1_000.0,
        ),
        ("al_runtime_source_fps", source_fps.clamp(1.0, 240.0)),
        (
            "al_runtime_random_seed",
            f64::from(schedule.random_graphic_seed),
        ),
        (
            "al_runtime_frame_inner_percent",
            video.frame_inner_perturbation_percent,
        ),
        (
            "al_runtime_frame_inter_percent",
            video.frame_inter_perturbation_percent,
        ),
        (
            "al_runtime_frame_probability_percent",
            advanced.frame_perturbation_probability_percent,
        ),
        (
            "al_runtime_slice_length_seconds",
            advanced.slice_length_ms as f64 / 1_000.0,
        ),
        (
            "al_runtime_slice_interval_seconds",
            advanced.slice_trigger_interval_ms as f64 / 1_000.0,
        ),
        (
            "al_runtime_pip_jitter_px",
            advanced.picture_in_picture_pixel_jitter_px,
        ),
        (
            "al_runtime_local_blur_interval_seconds",
            advanced.local_blur_interval_ms as f64 / 1_000.0,
        ),
        (
            "al_runtime_smoothing_enabled",
            enabled(advanced.transform_smoothing_enabled),
        ),
        (
            "al_runtime_smoothing_seconds",
            advanced.transform_smoothing_duration_ms as f64 / 1_000.0,
        ),
        (
            "al_runtime_highlight_enabled",
            enabled(advanced.highlight_perturbation_enabled),
        ),
        (
            "al_runtime_highlight_interval_seconds",
            advanced.highlight_perturbation_interval_ms as f64 / 1_000.0,
        ),
        (
            "al_runtime_async_rotation_enabled",
            enabled(advanced.asynchronous_rotation_enabled),
        ),
        (
            "al_runtime_async_rotation_min_degrees",
            advanced.asynchronous_rotation_min_degrees,
        ),
        (
            "al_runtime_async_rotation_max_degrees",
            advanced.asynchronous_rotation_max_degrees,
        ),
    ] {
        if !update.value.is_empty() {
            update.value.push(',');
        }
        update.value.push_str(option);
        update.value.push('=');
        update.value.push_str(&format_shader_number(value));
    }
    Ok(update)
}

fn format_shader_number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let mut formatted = format!("{value:.10}");
    while formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    formatted
}

#[derive(Debug, Clone)]
pub struct Gpu83VideoFilterPlan {
    pub serial_filter: String,
    pub applied_fields: Vec<String>,
    pub requires_variable_frame_rate: bool,
}

/// 保留给旧 FFmpeg 调用方的兼容适配器，不代表 mpv GPU83 能力已经全部生效。
/// 原子计划仍是唯一准入和字段证据源；字段集合与当前 83 项契约不完全一致时拒绝。
pub fn build_gpu83_video_filter(
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
    input_on_vulkan: bool,
    output_size: Option<(u32, u32)>,
) -> Option<Gpu83VideoFilterPlan> {
    let atomic = build_atomic_media_video_effect_plan(video, advanced).ok()?;
    let applied_fields = atomic_media_video_ui_applied_fields(&atomic)
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let applied_field_set = applied_fields
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_field_set = GPU83_FIELD_PATHS.iter().copied().collect::<BTreeSet<_>>();
    if applied_field_set != expected_field_set {
        return None;
    }

    let shader = gpu83_shader(video, advanced);
    let mut filters = Vec::with_capacity(4);
    if !input_on_vulkan {
        filters.extend(["format=nv12".to_owned(), "hwupload".to_owned()]);
    }
    let (width, height) = output_size
        .map(|(width, height)| (width.to_string(), height.to_string()))
        .unwrap_or_else(|| ("iw".to_owned(), "ih".to_owned()));
    filters.push(format!(
        "libplacebo=w={width}:h={height}:normalize_sar=true:brightness={:.6}:contrast={:.6}:saturation={:.6}:hue={:.10}:upscaler=bilinear:downscaler=bilinear:custom_shader_bin={}",
        (video.brightness_percent / 100.0).clamp(-1.0, 1.0),
        (video.contrast_percent / 100.0).clamp(0.0, 16.0),
        (video.saturation_percent / 100.0).clamp(0.0, 16.0),
        video
            .hue_rotation_degrees
            .to_radians()
            .clamp(-std::f64::consts::PI, std::f64::consts::PI),
        encode_binary_option(shader.as_bytes()),
    ));
    filters.extend(["hwdownload".to_owned(), "format=nv12".to_owned()]);
    Some(Gpu83VideoFilterPlan {
        serial_filter: filters.join(","),
        applied_fields,
        requires_variable_frame_rate: atomic.requires_variable_frame_rate,
    })
}

fn encode_binary_option(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn define(shader: &mut String, name: &str, value: f64) {
    let _ = writeln!(shader, "#define {name} {:.10}", value);
}

fn enabled(value: bool) -> f64 {
    if value {
        1.0
    } else {
        0.0
    }
}

fn gpu83_shader(video: &VideoEffectParams, advanced: &AdvancedEffectParams) -> String {
    let mut shader = String::with_capacity(12 * 1024);
    shader.push_str(
        "//!PARAM PTS\n//!TYPE float\n0.0\n\n//!HOOK MAIN\n//!BIND HOOKED\n//!DESC AutoLive legacy GPU filter\n//!WIDTH HOOKED.w\n//!HEIGHT HOOKED.h\n//!WHEN 1\n",
    );
    for (name, value) in [
        ("P_BLUR", video.blur_radius_px),
        ("P_SHARP", video.sharpen_percent / 100.0),
        ("P_NOISE", video.noise_percent / 100.0),
        ("P_DETAIL", video.detail_enhancement_percent / 100.0),
        ("P_CROP_SMOOTH", video.crop_edge_smoothing),
        ("P_FPS_JITTER", video.frame_rate_jitter_percent / 100.0),
        ("P_FPS_FREQ", video.frame_rate_perturbation_frequency_hz),
        ("P_FPS_AMP", video.frame_rate_perturbation_amplitude_fps),
        ("P_SCALE", video.pixel_scale_percent / 100.0),
        ("P_PIXEL_JITTER", video.pixel_jitter_px),
        ("P_DYNAMIC_CROP", video.dynamic_crop_percent / 100.0),
        ("P_INNER", video.frame_inner_perturbation_percent / 100.0),
        ("P_INTER", video.frame_inter_perturbation_percent / 100.0),
        ("P_SPACE_X", video.space_x_offset_px),
        ("P_SPACE_Y", video.space_y_offset_px),
        ("P_ROTATION", video.rotation_degrees.to_radians()),
        ("P_VIGNETTE", video.vignette_percent / 100.0),
        ("P_HIGHLIGHTS", video.highlights_percent / 100.0),
        ("P_SHADOWS", video.shadows_percent / 100.0),
        ("P_RED_LOCK", enabled(video.red_channel_lock_enabled)),
        ("P_EDGE_SOFT", video.edge_softness_percent / 100.0),
        ("P_REPAIR_ON", enabled(video.image_repair_enabled)),
        ("P_REPAIR", video.image_repair_strength_percent / 100.0),
        ("P_FPS_LOCK", enabled(video.frame_rate_lock_enabled)),
        ("A_TARGET", advanced.target_frequency_hz.unwrap_or(65.0)),
        (
            "A_CORE",
            advanced
                .core_frequency_hz
                .or(advanced.target_frequency_hz)
                .unwrap_or(65.0),
        ),
        ("A_WAVE_INTENSITY", advanced.wave_intensity),
        ("A_WAVE_LEVEL", advanced.wave_level),
        ("A_GRAINS", f64::from(advanced.wave_grain_count)),
        ("A_EQ", advanced.dynamic_eq_threshold / 20.0),
        ("A_CHANNEL", advanced.channel_offset_percent / 100.0),
        ("A_DIM", f64::from(advanced.space_dimension)),
        ("A_FREQ_X", advanced.frequency_space_x_offset_px),
        ("A_FREQ_Y", advanced.frequency_space_y_offset_px),
        (
            "A_FRAME_PROB",
            advanced.frame_perturbation_probability_percent / 100.0,
        ),
        (
            "A_GRAPHIC_ALPHA",
            advanced.random_graphic_opacity_percent / 100.0,
        ),
        ("A_GRAPHIC_SIZE", advanced.random_graphic_size_px),
        ("A_FACE_COUNT", f64::from(advanced.abstract_face_count)),
        ("A_FACE_SIZE", advanced.abstract_face_size_percent / 100.0),
        (
            "A_FACE_ALPHA",
            advanced.abstract_face_opacity_percent / 100.0,
        ),
        ("A_OVERLAY_OFFSET", advanced.overlay_offset_px),
        ("A_SLICE_LENGTH", advanced.slice_length_ms as f64 / 1_000.0),
        ("A_SLICE_MIN", advanced.slice_min_length_ms as f64 / 1_000.0),
        (
            "A_SLICE_INTERVAL",
            advanced.slice_trigger_interval_ms as f64 / 1_000.0,
        ),
        ("A_GRAPHIC_ON", enabled(advanced.random_graphic_enabled)),
        ("A_GRAPHIC_COUNT", f64::from(advanced.random_graphic_count)),
        ("A_PIP_ON", enabled(advanced.picture_in_picture_enabled)),
        (
            "A_PIP_SCALE",
            advanced.picture_in_picture_scale_percent / 100.0,
        ),
        (
            "A_PIP_ALPHA",
            advanced.picture_in_picture_opacity_percent / 100.0,
        ),
        (
            "A_PIP_ROT",
            advanced.picture_in_picture_rotation_degrees.to_radians(),
        ),
        ("A_PIP_JITTER", advanced.picture_in_picture_pixel_jitter_px),
        (
            "A_PIP_LOCK",
            enabled(advanced.picture_in_picture_timeline_locked),
        ),
        ("A_LOCAL_BLUR_ON", enabled(advanced.local_blur_enabled)),
        (
            "A_LOCAL_BLUR_REGION",
            advanced.local_blur_region_percent / 100.0,
        ),
        ("A_LOCAL_BLUR", advanced.local_blur_radius_px),
        (
            "A_LOCAL_BLUR_INTERVAL",
            advanced.local_blur_interval_ms as f64 / 1_000.0,
        ),
        ("A_EDGE_FILL_ON", enabled(advanced.edge_fill_enabled)),
        ("A_EDGE_FEATHER", advanced.edge_feather_percent / 100.0),
        ("A_SMOOTH_ON", enabled(advanced.transform_smoothing_enabled)),
        (
            "A_SMOOTH_TIME",
            advanced.transform_smoothing_duration_ms as f64 / 1_000.0,
        ),
        (
            "A_HIGHLIGHT_ON",
            enabled(advanced.highlight_perturbation_enabled),
        ),
        (
            "A_HIGHLIGHT_INTERVAL",
            advanced.highlight_perturbation_interval_ms as f64 / 1_000.0,
        ),
        (
            "A_ASYNC_ON",
            enabled(advanced.asynchronous_rotation_enabled),
        ),
        (
            "A_ASYNC_MIN",
            advanced.asynchronous_rotation_min_degrees.to_radians(),
        ),
        (
            "A_ASYNC_MAX",
            advanced.asynchronous_rotation_max_degrees.to_radians(),
        ),
    ] {
        define(&mut shader, name, value);
    }
    for (index, frequency) in [
        65, 92, 131, 188, 267, 381, 544, 777, 1_110, 1_585, 2_263, 20_000,
    ]
    .into_iter()
    .enumerate()
    {
        define(
            &mut shader,
            &format!("A_BAND_{index}"),
            advanced
                .band_weights
                .get(&frequency)
                .copied()
                .unwrap_or(1.0),
        );
    }
    shader.push_str(
        r#"float h12(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453123);}
mat2 rot(float a){float c=cos(a),s=sin(a);return mat2(c,-s,s,c);}
vec4 hook(){
 float fr=float(frame); float timing=PTS;
 float bands=(A_BAND_0*sin(timing*65.0)+A_BAND_1*sin(timing*92.0)+A_BAND_2*sin(timing*131.0)+A_BAND_3*sin(timing*188.0)+A_BAND_4*sin(timing*267.0)+A_BAND_5*sin(timing*381.0)+A_BAND_6*sin(timing*544.0)+A_BAND_7*sin(timing*777.0)+A_BAND_8*sin(timing*1110.0)+A_BAND_9*sin(timing*1585.0)+A_BAND_10*sin(timing*2263.0)+A_BAND_11*sin(timing*20000.0))/12.0;
 float carrier=sin(6.2831853*timing*A_TARGET)+cos(6.2831853*timing*A_CORE);
 float transitionWeight=mix(1.0,smoothstep(0.0,max(A_SMOOTH_TIME,0.0001),mod(timing,max(A_SLICE_INTERVAL,0.0001))),A_SMOOTH_ON);
 float asyncAngle=mix(0.0,mix(A_ASYNC_MIN,A_ASYNC_MAX,0.5+0.5*sin(timing/max(A_SLICE_INTERVAL,0.0001)))*transitionWeight,A_ASYNC_ON);
 vec2 uv=HOOKED_pos-0.5; uv=rot(-(P_ROTATION+asyncAngle))*uv;
 float crop=1.0-max(P_DYNAMIC_CROP,0.0); uv/=max(crop*P_SCALE,0.01);
 vec2 px=HOOKED_pt; float rnd=h12(vec2(fr,A_GRAINS));
 uv+=px*vec2(P_SPACE_X+A_FREQ_X*A_DIM,P_SPACE_Y+A_FREQ_Y*max(A_DIM-1.0,0.0));
 uv+=px*P_PIXEL_JITTER*vec2(sin(fr*1.618),cos(fr*1.414));
 uv+=px*(P_INNER*carrier+A_FRAME_PROB*step(rnd, A_FRAME_PROB)+P_INTER*(rnd-0.5))*HOOKED_size;
 uv+=px*A_WAVE_INTENSITY*A_WAVE_LEVEL*(carrier+bands)*vec2(1.0,A_DIM-1.0);
 uv=mix(clamp(uv,-0.5,0.5),uv,smoothstep(0.0,1.0,P_CROP_SMOOTH))+0.5;
 vec2 tap=px*max(P_BLUR+A_LOCAL_BLUR,1.0); vec4 c=HOOKED_tex(clamp(uv,0.0,1.0));
 vec4 n=(HOOKED_tex(clamp(uv+vec2(tap.x,0),0.0,1.0))+HOOKED_tex(clamp(uv-vec2(tap.x,0),0.0,1.0))+HOOKED_tex(clamp(uv+vec2(0,tap.y),0.0,1.0))+HOOKED_tex(clamp(uv-vec2(0,tap.y),0.0,1.0)))*0.25;
 float local=step(length(uv-vec2(0.5+A_OVERLAY_OFFSET*px.x,0.5)),A_LOCAL_BLUR_REGION*0.5)*step(mod(timing,max(A_LOCAL_BLUR_INTERVAL,0.001)),A_SLICE_LENGTH);
 c=mix(c,n,clamp(P_BLUR/8.0+A_LOCAL_BLUR_ON*local*clamp(A_LOCAL_BLUR/16.0,0.0,1.0),0.0,1.0));
 c.rgb+=clamp(P_SHARP+P_DETAIL,0.0,1.5)*(c.rgb-n.rgb); c.rgb=mix(c.rgb,n.rgb,P_REPAIR_ON*P_REPAIR*0.35);
 float lum=dot(c.rgb,vec3(0.2126,0.7152,0.0722)); vec3 original=c.rgb;
 c.rgb+=vec3(P_SHADOWS*(1.0-lum)+P_HIGHLIGHTS*lum)*0.12;
 c.r=mix(c.r,original.r,P_RED_LOCK); c.rgb+=vec3(A_CHANNEL,-A_CHANNEL*0.5,A_CHANNEL*0.25)*(carrier+A_EQ*bands);
 c.rgb+=(h12(uv*HOOKED_size+fr)-0.5)*(P_NOISE+A_WAVE_LEVEL/max(A_GRAINS,1.0));
 float vig=smoothstep(0.8,0.2,length(uv-0.5)); c.rgb*=mix(1.0,vig,P_VIGNETTE);
 float edge=min(min(uv.x,uv.y),min(1.0-uv.x,1.0-uv.y)); c.rgb=mix(n.rgb,c.rgb,smoothstep(0.0,max(P_EDGE_SOFT*0.2,0.0001),edge));
 float activeSlice=step(mod(timing,max(A_SLICE_INTERVAL+A_SLICE_MIN*0.001,0.001)),A_SLICE_LENGTH);
 vec2 gp=fract((uv+vec2(A_OVERLAY_OFFSET)*px)*max(A_GRAPHIC_COUNT,1.0))-0.5;
 float graphic=A_GRAPHIC_ON*activeSlice*step(length(gp),A_GRAPHIC_SIZE*max(px.x,px.y)); c.rgb=mix(c.rgb,vec3(rnd,1.0-rnd,0.5),graphic*A_GRAPHIC_ALPHA);
 float face=step(length((uv-0.5)*vec2(1.0,1.3)),A_FACE_SIZE)*min(A_FACE_COUNT,1.0); c.rgb=mix(c.rgb,vec3(lum),face*A_FACE_ALPHA);
 vec2 pipuv=rot(-A_PIP_ROT)*(uv-vec2(0.75,0.75))+0.5+px*A_PIP_JITTER*vec2(sin(fr),cos(fr)); vec4 pip=HOOKED_tex(clamp((pipuv-0.5)/max(A_PIP_SCALE,0.01)+0.5,0.0,1.0));
 float pipbox=step(max(abs(uv.x-0.75),abs(uv.y-0.75)),A_PIP_SCALE*0.5)*A_PIP_ON*activeSlice; c=mix(c,pip,pipbox*A_PIP_ALPHA*mix(0.75,1.0,A_PIP_LOCK));
 float feather=smoothstep(0.0,max(A_EDGE_FEATHER*0.2,0.0001),edge); c.rgb=mix(n.rgb,c.rgb,mix(1.0,feather,A_EDGE_FILL_ON));
 c.rgb+=A_HIGHLIGHT_ON*step(mod(timing,max(A_HIGHLIGHT_INTERVAL,0.001)),0.05)*0.004;
 return clamp(c,0.0,1.0);
}
"#,
    );
    shader
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime_video_backend::VideoPlanIdentity;
    use std::collections::BTreeSet;

    fn active_schedule() -> VideoFrameSchedule {
        VideoFrameSchedule {
            identity: VideoPlanIdentity {
                session_id: 1,
                playback_generation: 2,
                source_revision: 3,
                parameter_revision: 4,
                sequence: 5,
            },
            schedule_epoch: 1,
            media_pts_ms: 1_000,
            epoch_elapsed_ms: 1_000,
            frame_index: 30,
            target_fps: 30.0,
            base_video_speed: 1.0,
            frame_rate_locked: false,
            frame_inner_active: true,
            frame_inter_active: true,
            frame_probability_active: true,
            slice_active: true,
            random_graphic_seed: 12_345_678,
            local_blur_active: true,
            highlight_active: true,
            pip_jitter_x_px: 1.25,
            pip_jitter_y_px: -2.5,
            asynchronous_rotation_degrees: 3.125,
            transform_easing: 0.5,
        }
    }

    #[test]
    fn mapping_is_the_exact_unique_gpu83_contract() {
        validate_gpu83_mapping_table(&GPU83_PARAMETER_MAPPINGS)
            .expect("GPU83 mapping must equal the exact contract");

        let expected = GPU83_FIELD_PATHS.iter().copied().collect::<BTreeSet<_>>();
        let actual = GPU83_PARAMETER_MAPPINGS
            .iter()
            .map(|mapping| mapping.field_path)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
        assert!(!actual.contains("video.horizontal_flip_enabled"));
        assert!(!actual.contains("video.vertical_flip_enabled"));
        assert_eq!(
            actual
                .iter()
                .filter(|field| field.starts_with("advanced.band_weights."))
                .count(),
            12
        );
        let capability_counts = GPU83_PARAMETER_MAPPINGS.iter().fold(
            (0, 0, 0, 0),
            |(shader, scheduler, color, history), mapping| match mapping.capability {
                Gpu83ParameterCapability::ShaderParameter => {
                    (shader + 1, scheduler, color, history)
                }
                Gpu83ParameterCapability::ScheduledParameter => {
                    (shader, scheduler + 1, color, history)
                }
                Gpu83ParameterCapability::Unavailable(ALGORITHM_NOT_VERIFIED) => {
                    (shader, scheduler, color + 1, history)
                }
                Gpu83ParameterCapability::Unavailable(REQUIRES_HISTORY_TEXTURE) => {
                    (shader, scheduler, color, history + 1)
                }
                Gpu83ParameterCapability::Unavailable(_) => unreachable!("未知 capability 原因"),
            },
        );
        assert_eq!(capability_counts, (61, 18, 2, 2));
        assert_eq!(
            GPU83_PARAMETER_MAPPINGS
                .iter()
                .filter(|mapping| {
                    mapping.capability
                        == Gpu83ParameterCapability::Unavailable(REQUIRES_HISTORY_TEXTURE)
                })
                .map(|mapping| mapping.field_path)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "advanced.picture_in_picture_timeline_locked",
                "advanced.slice_min_length_ms",
            ])
        );
        assert_eq!(
            GPU83_PARAMETER_MAPPINGS
                .iter()
                .find(|mapping| {
                    mapping.field_path == "advanced.picture_in_picture_pixel_jitter_px"
                })
                .map(|mapping| mapping.capability),
            Some(Gpu83ParameterCapability::ScheduledParameter)
        );
    }

    #[test]
    fn renderer_color_management_contract_stays_fail_closed() {
        let color_mappings = GPU83_PARAMETER_MAPPINGS
            .iter()
            .filter(|mapping| {
                matches!(
                    mapping.field_path,
                    "video.color_space_conversion_strength_percent"
                        | "video.color_space_conversion_enabled"
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(color_mappings.len(), 2);
        assert!(color_mappings.iter().all(|mapping| {
            mapping.capability == Gpu83ParameterCapability::Unavailable(ALGORITHM_NOT_VERIFIED)
                && mapping.execution_class == Gpu83ExecutionClass::PixelShader
        }));

        for forbidden in [
            "//!PARAM al_color_space_strength_percent",
            "//!PARAM al_color_space_enabled",
            "color.rgb.gbr",
            "color.rgb.brg",
            "color_space_weight",
        ] {
            assert!(
                !GPU83_SHADER_SOURCE.contains(forbidden),
                "未定义 primaries/TRC/range/输出标签时禁止固定 RGB 变换: {forbidden}"
            );
        }

        let legacy_shader = gpu83_shader(
            &VideoEffectParams {
                color_space_conversion_enabled: true,
                color_space_conversion_strength_percent: 100.0,
                ..VideoEffectParams::default()
            },
            &AdvancedEffectParams::default(),
        );
        for forbidden in ["P_COLOR_SPACE", "vec3(lum)+vec3(c.r-c.b"] {
            assert!(
                !legacy_shader.contains(forbidden),
                "旧兼容 shader 也不得保留固定色偏伪语义: {forbidden}"
            );
        }

        let video = VideoEffectParams {
            color_space_conversion_enabled: true,
            color_space_conversion_strength_percent: 100.0,
            ..VideoEffectParams::default()
        };
        let snapshot = build_gpu83_shader_snapshot(&video, &AdvancedEffectParams::default())
            .expect("合法输入必须形成 fail-closed 能力快照");
        assert!(snapshot
            .unavailable_fields
            .contains(&"video.color_space_conversion_enabled"));
        assert!(snapshot
            .unavailable_fields
            .contains(&"video.color_space_conversion_strength_percent"));
        assert!(!snapshot.shader_options.contains("al_color_space_enabled="));
        assert!(!snapshot
            .shader_options
            .contains("al_color_space_strength_percent="));
    }

    #[test]
    fn blocked_phase3_fields_never_leak_into_base_or_scheduled_shader_snapshots() {
        let video = VideoEffectParams {
            color_space_conversion_enabled: true,
            color_space_conversion_strength_percent: 37.5,
            ..VideoEffectParams::default()
        };
        let advanced = AdvancedEffectParams {
            picture_in_picture_enabled: true,
            picture_in_picture_timeline_locked: false,
            ..AdvancedEffectParams::default()
        };
        let snapshot = build_gpu83_shader_snapshot(&video, &advanced)
            .expect("合法但未准入的字段必须形成 fail-closed 快照");
        assert_eq!(
            snapshot.unavailable_fields,
            vec![
                "video.color_space_conversion_strength_percent",
                "video.color_space_conversion_enabled",
                "advanced.slice_min_length_ms",
                "advanced.picture_in_picture_timeline_locked",
            ]
        );

        let scheduled = build_gpu83_scheduled_shader_update(&video, &advanced, &active_schedule())
            .expect("调度快照不得绕过 Phase 3 未准入门禁");
        assert!(scheduled.value.contains("al_runtime_pip_jitter_px="));
        for forbidden_option in [
            "al_color_space_strength_percent=",
            "al_color_space_enabled=",
            "al_slice_min_length_ms=",
            "al_pip_timeline_locked=",
        ] {
            assert!(!snapshot.shader_options.contains(forbidden_option));
            assert!(!scheduled.value.contains(forbidden_option));
        }
    }

    #[test]
    fn mapping_validator_detects_missing_and_duplicate_entries() {
        let mut malformed = GPU83_PARAMETER_MAPPINGS.to_vec();
        let removed = malformed.pop().expect("mapping fixture is non-empty");
        malformed.push(malformed[0]);

        let errors = validate_gpu83_mapping_table(&malformed)
            .expect_err("malformed mapping must be rejected");
        assert!(errors
            .iter()
            .any(|error| error == "duplicate_field:video.brightness_percent"));
        assert!(errors
            .iter()
            .any(|error| error == "duplicate_option:al_brightness_percent"));
        assert!(errors
            .iter()
            .any(|error| error == &format!("missing_field:{}", removed.field_path)));
    }

    #[test]
    fn snapshot_rejects_non_finite_and_missing_band_values() {
        let video = VideoEffectParams {
            brightness_percent: f64::NAN,
            ..VideoEffectParams::default()
        };
        let error = build_gpu83_shader_snapshot(&video, &AdvancedEffectParams::default())
            .expect_err("NaN must be rejected before serialization");
        assert_eq!(error.field, "video.brightness_percent");
        assert_eq!(error.code, "non_finite_value");

        let mut advanced = AdvancedEffectParams::default();
        advanced.band_weights.remove(&65);
        let error = build_gpu83_shader_snapshot(&VideoEffectParams::default(), &advanced)
            .expect_err("missing fixed visual band must be rejected");
        assert_eq!(error.field, "advanced.band_weights.65");
        assert_eq!(error.code, "missing_value");
    }

    #[test]
    fn one_snapshot_produces_one_atomic_shader_options_update() {
        let snapshot = build_gpu83_shader_snapshot(
            &VideoEffectParams::default(),
            &AdvancedEffectParams::default(),
        )
        .expect("default parameters are valid");
        let update = snapshot.mpv_property_update();
        let available_count = GPU83_PARAMETER_MAPPINGS
            .iter()
            .filter(|mapping| mapping.capability == Gpu83ParameterCapability::ShaderParameter)
            .count();

        assert_eq!(snapshot.entries.len(), GPU83_PARAMETER_COUNT);
        assert!(!snapshot.is_fully_available());
        assert_eq!(update.property, "glsl-shader-opts");
        assert_eq!(update.value.split(',').count(), available_count);
        assert!(!update.value.contains("NaN"));
        assert!(!update.value.contains("inf"));
        assert!(update.value.contains("al_brightness_percent=0"));
        assert!(update.value.contains("al_target_frequency_hz=0"));
        assert!(update.value.contains("al_core_frequency_hz=0"));
    }

    #[test]
    fn every_cycle_snapshot_appends_static_pts_inputs() {
        let video = VideoEffectParams::default();
        let advanced = AdvancedEffectParams::default();
        let base = build_gpu83_shader_snapshot(&video, &advanced)
            .expect("default parameters are valid")
            .mpv_property_update();
        let scheduled = build_gpu83_scheduled_shader_update(&video, &advanced, &active_schedule())
            .expect("default parameters must produce a complete cycle snapshot");

        assert_eq!(scheduled.property, base.property);
        assert!(scheduled.value.starts_with(&format!("{},", base.value)));
        assert!(scheduled.value.contains("al_runtime_epoch_start_seconds=1"));
        assert!(scheduled.value.contains("al_runtime_source_fps=30"));
        assert!(scheduled.value.contains("al_runtime_frame_inner_percent=0"));
    }

    #[test]
    fn active_schedule_appends_runtime_options_once_in_fixed_order() {
        let video = VideoEffectParams {
            frame_inner_perturbation_percent: 1.0,
            frame_inter_perturbation_percent: 2.0,
            ..VideoEffectParams::default()
        };
        let advanced = AdvancedEffectParams {
            frame_perturbation_probability_percent: 10.0,
            random_graphic_enabled: true,
            picture_in_picture_enabled: true,
            picture_in_picture_pixel_jitter_px: 4.0,
            local_blur_enabled: true,
            highlight_perturbation_enabled: true,
            asynchronous_rotation_enabled: true,
            transform_smoothing_enabled: true,
            ..AdvancedEffectParams::default()
        };
        let base = build_gpu83_shader_snapshot(&video, &advanced)
            .expect("configured parameters are valid")
            .mpv_property_update();
        let update = build_gpu83_scheduled_shader_update(&video, &advanced, &active_schedule())
            .expect("active schedule is valid");
        let prefix = format!("{},", base.value);
        let runtime = update
            .value
            .strip_prefix(&prefix)
            .expect("runtime options must follow the base snapshot");

        assert_eq!(
            runtime.split(',').collect::<Vec<_>>(),
            vec![
                "al_runtime_epoch_start_seconds=1",
                "al_runtime_source_fps=30",
                "al_runtime_random_seed=12345678",
                "al_runtime_frame_inner_percent=1",
                "al_runtime_frame_inter_percent=2",
                "al_runtime_frame_probability_percent=10",
                "al_runtime_slice_length_seconds=5",
                "al_runtime_slice_interval_seconds=15",
                "al_runtime_pip_jitter_px=4",
                "al_runtime_local_blur_interval_seconds=10",
                "al_runtime_smoothing_enabled=1",
                "al_runtime_smoothing_seconds=0.8",
                "al_runtime_highlight_enabled=1",
                "al_runtime_highlight_interval_seconds=10",
                "al_runtime_async_rotation_enabled=1",
                "al_runtime_async_rotation_min_degrees=-1",
                "al_runtime_async_rotation_max_degrees=1",
            ]
        );
    }

    #[test]
    fn asynchronous_rotation_is_committed_as_static_pts_input() {
        let advanced = AdvancedEffectParams {
            asynchronous_rotation_enabled: true,
            transform_smoothing_enabled: false,
            ..AdvancedEffectParams::default()
        };
        let update = build_gpu83_scheduled_shader_update(
            &VideoEffectParams::default(),
            &advanced,
            &active_schedule(),
        )
        .expect("asynchronous rotation schedule is valid");

        assert!(update.value.contains("al_runtime_async_rotation_enabled=1"));
        assert!(update
            .value
            .contains("al_runtime_async_rotation_min_degrees=-1"));
        assert!(update
            .value
            .contains("al_runtime_async_rotation_max_degrees=1"));
        assert!(update.value.contains("al_runtime_smoothing_enabled=0"));
    }

    #[test]
    fn scheduled_snapshot_rejects_non_finite_runtime_values() {
        let mut schedule = active_schedule();
        schedule.target_fps = f64::NAN;
        let error = build_gpu83_scheduled_shader_update(
            &VideoEffectParams::default(),
            &AdvancedEffectParams::default(),
            &schedule,
        )
        .expect_err("non-finite schedule values must be rejected before serialization");

        assert_eq!(error.field, "schedule.source_fps");
        assert_eq!(error.code, "non_finite_value");

        let mut schedule = active_schedule();
        schedule.random_graphic_seed = MAX_RANDOM_GRAPHIC_SEED + 1;
        let error = build_gpu83_scheduled_shader_update(
            &VideoEffectParams::default(),
            &AdvancedEffectParams::default(),
            &schedule,
        )
        .expect_err("shader seed must stay exactly representable by f32");

        assert_eq!(error.field, "schedule.random_graphic_seed");
        assert_eq!(error.code, "out_of_range");
    }

    #[test]
    fn packaged_shader_has_valid_mpv_compute_hook_structure() {
        let source = GPU83_SHADER_SOURCE;
        let hook_start = source.find("//!HOOK MAIN").expect("MAIN hook metadata");
        let function_start = source[hook_start..]
            .find("void hook()")
            .map(|offset| hook_start + offset)
            .expect("compute hook function");
        for metadata in [
            "//!BIND HOOKED",
            "//!WIDTH HOOKED.w",
            "//!HEIGHT HOOKED.h",
            "//!WHEN 1",
            "//!COMPUTE 8 8",
        ] {
            let position = source[hook_start..]
                .find(metadata)
                .map(|offset| hook_start + offset)
                .unwrap_or_else(|| panic!("missing shader metadata {metadata}"));
            assert!(
                position < function_start,
                "{metadata} must precede hook body"
            );
        }
        assert!(!source.contains("fr/30"));
        assert!(!source.contains("/30.0"));
        assert!(!source
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|token| token == "smooth"));
        assert!(source.contains("imageStore(out_image"));
    }

    #[test]
    fn shader_declares_exactly_the_available_and_static_pts_options() {
        let declared = GPU83_SHADER_SOURCE
            .lines()
            .filter_map(|line| line.strip_prefix("//!PARAM "))
            .filter(|name| *name != "PTS" && !name.starts_with("al_runtime_"))
            .collect::<BTreeSet<_>>();
        let runtime = GPU83_SHADER_SOURCE
            .lines()
            .filter_map(|line| line.strip_prefix("//!PARAM "))
            .filter(|name| name.starts_with("al_runtime_"))
            .collect::<BTreeSet<_>>();
        let available = GPU83_PARAMETER_MAPPINGS
            .iter()
            .filter(|mapping| mapping.capability == Gpu83ParameterCapability::ShaderParameter)
            .map(|mapping| mapping.shader_option)
            .collect::<BTreeSet<_>>();
        assert_eq!(declared, available);
        assert_eq!(runtime.len(), 19);
        assert!(runtime.contains("al_runtime_plan_hi"));
        assert!(runtime.contains("al_runtime_plan_lo"));
        assert!(GPU83_SHADER_SOURCE.contains("max(PTS - al_runtime_epoch_start_seconds"));
        for removed_per_frame_output in [
            "al_runtime_frame_inner_active",
            "al_runtime_pip_jitter_x_px",
            "al_runtime_async_rotation_degrees",
            "al_runtime_transform_easing",
        ] {
            assert!(!runtime.contains(removed_per_frame_output));
        }
    }
}
