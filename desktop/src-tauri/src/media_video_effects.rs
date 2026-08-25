//! 正式视频/高级视觉参数到单输入 FFmpeg `-vf` 的映射。
//!
//! 这里只生成已确认可由打包 FFmpeg 执行、并会真实改变输出的滤镜。调用方仍需先执行
//! 参数模型校验；默认值保持旁路，实际进入滤镜图的字段记录在 `applied_fields`。

use crate::media_effect_params::{AdvancedEffectParams, VideoEffectParams};
use std::convert::Infallible;

const MIN_VISUAL_FREQUENCY_HZ: f64 = 65.0;
const MAX_VISUAL_FREQUENCY_HZ: f64 = 20_000.0;
const MIN_SPATIAL_CYCLES: f64 = 1.0;
const MAX_SPATIAL_CYCLES: f64 = 32.0;
const MIN_EFFECTIVE_OVERLAY_ALPHA: f64 = 2.0 / 255.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaVideoEffectPlan {
    pub filters: Vec<String>,
    pub applied_fields: Vec<&'static str>,
    /// 帧率微扰会生成 VFR 时间戳，调用方必须为输出增加 `-fps_mode vfr`。
    pub requires_variable_frame_rate: bool,
    /// 动态裁剪在本计划中连同边缘插值一起生成；调用方不得再追加旧裁剪滤镜。
    pub replaces_base_dynamic_crop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaVideoComplexEffectPlan {
    /// 完整 `-filter_complex` 视频子图，输入固定为 `[0:v:0]`，输出固定为 `[vout]`。
    pub graph: String,
}

impl MediaVideoEffectPlan {
    pub fn filter_chain(&self) -> String {
        self.filters.join(",")
    }

    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

/// 保留旧调用签名的不可构造错误类型；正式视频字段现已全部映射。
pub type UnsupportedMediaVideoEffects = Infallible;

pub fn build_media_video_effect_plan(
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
) -> Result<MediaVideoEffectPlan, UnsupportedMediaVideoEffects> {
    let mut plan = MediaVideoEffectPlan {
        filters: Vec::new(),
        applied_fields: Vec::new(),
        requires_variable_frame_rate: false,
        replaces_base_dynamic_crop: false,
    };

    append_crop_with_smoothing(&mut plan, video);
    append_inter_frame_perturbation(&mut plan, video);
    append_channel_offset(&mut plan, advanced);
    append_spatial_modulation(&mut plan, video, advanced);
    append_probability_perturbation(&mut plan, advanced);
    append_graphics(&mut plan, advanced);
    append_edge_fill(&mut plan, advanced);
    append_highlight_perturbation(&mut plan, advanced);
    append_asynchronous_rotation(&mut plan, advanced);
    append_frame_rate_perturbation(&mut plan, video);
    append_frame_rate_lock(&mut plan, video);
    append_base_effect_applied_fields(&mut plan, video);
    append_complex_effect_applied_fields(&mut plan, video, advanced);

    Ok(plan)
}

/// 为需要分支/合成的效果构建视频子图。基础串行滤镜由调用方传入，避免在两个模块
/// 重复维护亮度、裁剪等既有映射。
pub fn build_media_video_complex_effect_plan(
    base_filter_chain: &str,
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
) -> Option<MediaVideoComplexEffectPlan> {
    let color_conversion = video.color_space_conversion_enabled
        && video.color_space_conversion_strength_percent > f64::EPSILON;
    let local_blur = advanced.local_blur_enabled;
    let picture_in_picture = advanced.picture_in_picture_enabled
        && advanced.picture_in_picture_opacity_percent > f64::EPSILON;
    let edge_fill_feather = advanced.edge_fill_enabled && advanced.edge_feather_percent > 0.0;
    let edge_softness = video.edge_softness_percent > 0.0;
    if !color_conversion
        && !local_blur
        && !picture_in_picture
        && !edge_fill_feather
        && !edge_softness
    {
        return None;
    }

    let base_filter_chain = if base_filter_chain.is_empty() {
        "null"
    } else {
        base_filter_chain
    };
    let mut parts = vec![format!(
        "[0:v:0]{base_filter_chain},format=yuv420p[vstage0]"
    )];
    let mut current = "vstage0".to_owned();
    let mut stage = 1_u8;

    if color_conversion {
        let next = format!("vstage{stage}");
        let opacity = video.color_space_conversion_strength_percent / 100.0;
        parts.push(format!(
            "[{current}]split=2[vcolorbase][vcolorinput];[vcolorinput]colorspace=iall=bt601-6-625:all=bt709:format=yuv420p:fast=0[vconverted];[vcolorbase][vconverted]blend=all_opacity={opacity:.6}[{next}]"
        ));
        current = next;
        stage += 1;
    }

    if edge_fill_feather {
        let next = format!("vstage{stage}");
        let border = (2.0 + advanced.edge_feather_percent * 0.14)
            .round()
            .clamp(2.0, 16.0) as u8;
        let opacity =
            (advanced.edge_feather_percent / 100.0).clamp(MIN_EFFECTIVE_OVERLAY_ALPHA, 1.0);
        parts.push(format!(
            "[{current}]split=2[vedgebase][vedgefillinput];[vedgefillinput]fillborders=left={border}:right={border}:top={border}:bottom={border}:mode=smear[vedgefill];[vedgebase][vedgefill]blend=all_opacity={opacity:.6}[{next}]"
        ));
        current = next;
        stage += 1;
    }

    if edge_softness {
        let next = format!("vstage{stage}");
        let strength = video.edge_softness_percent / 100.0;
        let sigma = 0.5 + strength * 7.5;
        let opacity = strength.clamp(MIN_EFFECTIVE_OVERLAY_ALPHA, 1.0);
        parts.push(format!(
            "[{current}]split=2[vsoftbase][vsoftblurinput];[vsoftblurinput]gblur=sigma={sigma:.6}[vsoftblur];[vsoftbase][vsoftblur]blend=all_opacity={opacity:.6}[{next}]"
        ));
        current = next;
        stage += 1;
    }

    if local_blur {
        let next = format!("vstage{stage}");
        let region = advanced.local_blur_region_percent / 100.0;
        let start = 0.5 - region / 2.0;
        let end = 0.5 + region / 2.0;
        let interval = advanced.local_blur_interval_ms as f64 / 1_000.0;
        let active = (advanced.slice_length_ms as f64 / 1_000.0)
            .min(interval / 2.0)
            .max(0.1);
        let condition = format!(
            "between(X\\,W*{start:.6}\\,W*{end:.6})*between(Y\\,H*{start:.6}\\,H*{end:.6})*lt(mod(T\\,{interval:.6})\\,{active:.6})"
        );
        parts.push(format!(
            "[{current}]split=2[vlocalbase][vlocalinput];[vlocalinput]gblur=sigma={:.6}[vlocalblur];[vlocalbase][vlocalblur]blend=all_expr='if({condition}\\,B\\,A)'[{next}]",
            advanced.local_blur_radius_px
        ));
        current = next;
        stage += 1;
    }

    if picture_in_picture {
        let next = format!("vstage{stage}");
        let scale = advanced.picture_in_picture_scale_percent / 100.0;
        let jitter = advanced.picture_in_picture_pixel_jitter_px;
        let offset = advanced.overlay_offset_px;
        let mut pip_filters = Vec::new();
        let pip_start = if advanced.picture_in_picture_timeline_locked {
            0.0
        } else {
            picture_in_picture_start_seconds(advanced)
        };
        if !advanced.picture_in_picture_timeline_locked {
            pip_filters.push(format!("setpts=PTS-STARTPTS+{pip_start:.6}/TB"));
        }
        pip_filters.push(format!(
            "scale=trunc(iw*{scale:.6}/2)*2:trunc(ih*{scale:.6}/2)*2"
        ));
        if advanced.picture_in_picture_rotation_degrees.abs() > f64::EPSILON {
            let radians = advanced.picture_in_picture_rotation_degrees.to_radians();
            let angle = if advanced.transform_smoothing_enabled {
                format!(
                    "{radians:.10}*({})",
                    transform_easing_at(advanced, "t", pip_start)
                )
            } else {
                format!("{radians:.10}")
            };
            pip_filters.push(format!(
                "rotate=angle='{angle}':ow=rotw({radians:.10}):oh=roth({radians:.10}):fillcolor=black"
            ));
        }
        let opacity =
            (advanced.picture_in_picture_opacity_percent / 100.0).max(MIN_EFFECTIVE_OVERLAY_ALPHA);
        pip_filters.push(format!("format=rgba,colorchannelmixer=aa={opacity:.6}"));
        let schedule = slice_enable_at(advanced, pip_start);
        let easing = if advanced.transform_smoothing_enabled {
            format!("*({})", transform_easing_at(advanced, "t", pip_start))
        } else {
            String::new()
        };
        parts.push(format!(
            "[{current}]split=2[vpipbase][vpipinput];[vpipinput]{}[vpip];[vpipbase][vpip]overlay=x='W-w-12+{offset:.6}+sin(n*0.73)*{jitter:.6}{easing}':y='H-h-12+{offset:.6}+cos(n*0.91)*{jitter:.6}{easing}':eval=frame:shortest=0:eof_action=pass:repeatlast=0:enable='{schedule}'[{next}]",
            pip_filters.join(",")
        ));
        current = next;
    }

    parts.push(format!("[{current}]null[vout]"));
    Some(MediaVideoComplexEffectPlan {
        graph: parts.join(";"),
    })
}

fn append_crop_with_smoothing(plan: &mut MediaVideoEffectPlan, video: &VideoEffectParams) {
    if video.dynamic_crop_percent <= 0.0 {
        return;
    }
    let crop = video.dynamic_crop_percent / 100.0;
    let double_crop = crop * 2.0;
    let scale_flags = match video.crop_edge_smoothing {
        value if value < 0.25 => "neighbor",
        value if value < 0.5 => "bilinear",
        value if value < 0.75 => "bicubic",
        _ => "lanczos",
    };
    plan.filters.push(format!(
        "crop=iw*(1-{double_crop:.6}):ih*(1-{double_crop:.6}):iw*{crop:.6}+iw*{crop:.6}*sin(n*0.07):ih*{crop:.6}+ih*{crop:.6}*cos(n*0.09),scale=round(iw/(1-{double_crop:.6})/2)*2:round(ih/(1-{double_crop:.6})/2)*2:flags={scale_flags}"
    ));
    plan.applied_fields.push("video.dynamic_crop_percent");
    plan.applied_fields.push("video.crop_edge_smoothing");
    plan.replaces_base_dynamic_crop = true;
}

fn append_inter_frame_perturbation(plan: &mut MediaVideoEffectPlan, video: &VideoEffectParams) {
    if video.frame_inter_perturbation_percent > 0.0 {
        plan.filters.push(format!(
            "tblend=all_mode=average:all_opacity={:.6}",
            video.frame_inter_perturbation_percent / 100.0
        ));
        plan.applied_fields
            .push("video.frame_inter_perturbation_percent");
    }
}

fn append_channel_offset(plan: &mut MediaVideoEffectPlan, advanced: &AdvancedEffectParams) {
    if advanced.channel_offset_percent == 0.0 {
        return;
    }
    let (red_delta, blue_delta) = if advanced.channel_offset_percent.is_sign_positive() {
        (1, -1)
    } else {
        (-1, 1)
    };
    plan.filters.push(format!(
        "lutrgb=r='clip(val{red_delta:+}\\,0\\,255)':g='val':b='clip(val{blue_delta:+}\\,0\\,255)'"
    ));
    plan.applied_fields.push("advanced.channel_offset_percent");
}

fn append_spatial_modulation(
    plan: &mut MediaVideoEffectPlan,
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
) {
    let band_deltas = advanced
        .band_weights
        .iter()
        .filter_map(|(frequency_hz, weight)| {
            let delta = weight - 1.0;
            (delta.abs() > f64::EPSILON)
                .then_some((spatial_cycles(f64::from(*frequency_hz)), delta))
        })
        .collect::<Vec<_>>();
    let band_energy = band_deltas
        .iter()
        .map(|(_, delta)| delta.abs())
        .sum::<f64>();
    let weighted_cycles = (band_energy > f64::EPSILON).then(|| {
        band_deltas
            .iter()
            .map(|(cycles, delta)| cycles * delta.abs())
            .sum::<f64>()
            / band_energy
    });
    let band_phase = band_deltas
        .iter()
        .enumerate()
        .map(|(index, (_, delta))| delta * (index + 1) as f64)
        .sum::<f64>()
        * std::f64::consts::TAU;
    let target_cycles = advanced.target_frequency_hz.map(spatial_cycles);
    let band_cycles = target_cycles.or(weighted_cycles).map(|cycles| {
        if let (Some(target), Some(weighted)) = (target_cycles, weighted_cycles) {
            (target + (weighted - target) * band_energy.min(0.25))
                .clamp(MIN_SPATIAL_CYCLES, MAX_SPATIAL_CYCLES)
        } else {
            cycles
        }
    });
    let wave_active = advanced.wave_intensity > 0.0;
    let frame_inner_delta = frame_inner_perturbation_delta(video);
    if band_cycles.is_none()
        && !wave_active
        && advanced.wave_level <= 0.0
        && frame_inner_delta.is_none()
    {
        return;
    }

    let mut delta = format!("{:.6}", advanced.wave_level * 4.0);
    if let Some(frame_inner_delta) = &frame_inner_delta {
        delta.push_str(&format!("+({frame_inner_delta})"));
    }
    if band_cycles.is_some() || wave_active {
        let grain = f64::from(advanced.wave_grain_count);
        let cycles = band_cycles.unwrap_or(grain)
            + if wave_active {
                advanced.wave_intensity * grain
            } else {
                0.0
            };
        let mut position = if wave_active {
            format!("(X+{:.6})/W", advanced.frequency_space_x_offset_px)
        } else {
            "X/W".to_owned()
        };
        if advanced.space_dimension >= 2 {
            if wave_active {
                position.push_str(&format!(
                    "+(Y+{:.6})/H",
                    advanced.frequency_space_y_offset_px
                ));
            } else {
                position.push_str("+Y/H");
            }
        }
        let core_phase = advanced
            .core_frequency_hz
            .map(frequency_position)
            .unwrap_or(0.0)
            * std::f64::consts::TAU
            + band_phase;
        let time_phase = if wave_active && advanced.space_dimension == 3 {
            "+2*PI*T"
        } else {
            ""
        };
        let carrier = format!("sin(2*PI*{cycles:.8}*({position})+{core_phase:.8}{time_phase})");
        let band_limit = if band_cycles.is_some() {
            if advanced.dynamic_eq_threshold <= f64::EPSILON {
                20.0
            } else {
                advanced.dynamic_eq_threshold
            }
        } else {
            0.0
        };
        let band_amplitude = if band_cycles.is_some() {
            (1.0 + band_energy * 4.0).min(band_limit)
        } else {
            0.0
        };
        let wave_amplitude = advanced.wave_intensity * 12.0;
        let amplitude = band_amplitude + wave_amplitude;
        delta.push_str(&format!("+{amplitude:.8}*{carrier}"));
    }

    plan.filters.push(yuv_geq(
        &format!("clip(lum(X\\,Y)+{delta}\\,0\\,255)"),
        None,
    ));

    if !band_deltas.is_empty() {
        plan.applied_fields.push("advanced.band_weights");
    }
    if advanced.target_frequency_hz.is_some() {
        plan.applied_fields.push("advanced.target_frequency_hz");
    }
    if advanced.core_frequency_hz.is_some() {
        plan.applied_fields.push("advanced.core_frequency_hz");
    }
    if target_cycles.is_some() || !band_deltas.is_empty() {
        plan.applied_fields.push("advanced.dynamic_eq_threshold");
    }
    if advanced.wave_intensity > 0.0 {
        plan.applied_fields.push("advanced.wave_intensity");
        plan.applied_fields.push("advanced.wave_grain_count");
        plan.applied_fields.push("advanced.space_dimension");
        plan.applied_fields
            .push("advanced.frequency_space_x_offset_px");
        if advanced.space_dimension >= 2 {
            plan.applied_fields
                .push("advanced.frequency_space_y_offset_px");
        }
    }
    if advanced.wave_level > 0.0 {
        plan.applied_fields.push("advanced.wave_level");
    }
    if frame_inner_delta.is_some() {
        plan.applied_fields
            .push("video.frame_inner_perturbation_percent");
    }
}

fn frame_inner_perturbation_delta(video: &VideoEffectParams) -> Option<String> {
    if video.frame_inner_perturbation_percent <= 0.0 {
        return None;
    }
    // 随包 FFmpeg 的 noise 强度只接受整数；用每帧 ±1 码值像素密度保留小数百分比，
    // 避免 0.01–0.99% 被强制放大为 strength=1，并与其他亮度空间调制共用一次 geq。
    let density_threshold = (video.frame_inner_perturbation_percent * 1_000.0).round() as u64;
    let phase = density_threshold * 7_919 % 100_000;
    Some(format!(
        "if(lt(mod(X*73+Y*151+N*199+{phase}\\,100000)\\,{density_threshold})\\,if(eq(mod(X+Y+N\\,2)\\,0)\\,1\\,-1)\\,0)"
    ))
}

fn frequency_position(frequency_hz: f64) -> f64 {
    (frequency_hz / MIN_VISUAL_FREQUENCY_HZ).log2()
        / (MAX_VISUAL_FREQUENCY_HZ / MIN_VISUAL_FREQUENCY_HZ).log2()
}

fn spatial_cycles(frequency_hz: f64) -> f64 {
    MIN_SPATIAL_CYCLES
        + frequency_position(frequency_hz) * (MAX_SPATIAL_CYCLES - MIN_SPATIAL_CYCLES)
}

fn append_probability_perturbation(
    plan: &mut MediaVideoEffectPlan,
    advanced: &AdvancedEffectParams,
) {
    let probability = advanced.frame_perturbation_probability_percent;
    if probability <= 0.0 {
        return;
    }
    let strength = (probability / 2.0).ceil().clamp(1.0, 10.0) as u8;
    let threshold = (probability * 1_000.0).round() as u64;
    let phase = threshold * 7_919 % 100_000;
    // 与帧号互质的步长形成确定性十万帧采样；参数派生相位避免每轮都命中首帧。
    plan.filters.push(format!(
        "noise=alls={strength}:allf=t+u:enable='lt(mod(n*37+{phase}\\,100000)\\,{threshold})'"
    ));
    plan.applied_fields
        .push("advanced.frame_perturbation_probability_percent");
}

fn append_graphics(plan: &mut MediaVideoEffectPlan, advanced: &AdvancedEffectParams) {
    let schedule = slice_enable(advanced);
    let mut appended = false;

    if advanced.random_graphic_enabled && advanced.random_graphic_opacity_percent > 0.0 {
        let size = advanced.random_graphic_size_px;
        let offset = advanced.overlay_offset_px;
        let alpha =
            (advanced.random_graphic_opacity_percent / 100.0).max(MIN_EFFECTIVE_OVERLAY_ALPHA);
        for index in 0..advanced.random_graphic_count {
            let x_fraction = (17 + u32::from(index) * 37) % 89;
            let y_fraction = (29 + u32::from(index) * 53) % 89;
            plan.filters.push(format!(
                "drawbox=x='mod(iw*0.{x_fraction:02}+{offset:.6}+iw\\,max(iw-{size:.6}\\,1))':y='mod(ih*0.{y_fraction:02}+{offset:.6}+ih\\,max(ih-{size:.6}\\,1))':w={size:.6}:h={size:.6}:color=white@{alpha:.6}:t=fill:enable='{schedule}'"
            ));
        }
        plan.applied_fields.push("advanced.random_graphic_enabled");
        plan.applied_fields.push("advanced.random_graphic_count");
        plan.applied_fields
            .push("advanced.random_graphic_opacity_percent");
        plan.applied_fields.push("advanced.random_graphic_size_px");
        appended = true;
    }

    if advanced.abstract_face_count > 0 && advanced.abstract_face_opacity_percent > 0.0 {
        let alpha =
            (advanced.abstract_face_opacity_percent / 100.0).max(MIN_EFFECTIVE_OVERLAY_ALPHA);
        for index in 0..advanced.abstract_face_count {
            plan.filters
                .extend(abstract_face_filters(index, advanced, alpha, &schedule));
        }
        plan.applied_fields.push("advanced.abstract_face_count");
        plan.applied_fields
            .push("advanced.abstract_face_size_percent");
        plan.applied_fields
            .push("advanced.abstract_face_opacity_percent");
        appended = true;
    }

    if appended {
        append_overlay_schedule_fields(plan);
    }
}

fn append_edge_fill(plan: &mut MediaVideoEffectPlan, advanced: &AdvancedEffectParams) {
    if !advanced.edge_fill_enabled {
        return;
    }
    if advanced.edge_feather_percent <= 0.0 {
        plan.filters
            .push("fillborders=left=2:right=2:top=2:bottom=2:mode=smear".to_owned());
    }
    plan.applied_fields.push("advanced.edge_fill_enabled");
}

fn append_highlight_perturbation(plan: &mut MediaVideoEffectPlan, advanced: &AdvancedEffectParams) {
    if !advanced.highlight_perturbation_enabled {
        return;
    }
    let interval = advanced.highlight_perturbation_interval_ms as f64 / 1_000.0;
    plan.filters.push(format!(
        "lutyuv=y='clip(val+1\\,0\\,255)':enable='lt(mod(t\\,{interval:.6})\\,0.050000)'"
    ));
    plan.applied_fields
        .push("advanced.highlight_perturbation_enabled");
    plan.applied_fields
        .push("advanced.highlight_perturbation_interval_ms");
}

fn append_asynchronous_rotation(plan: &mut MediaVideoEffectPlan, advanced: &AdvancedEffectParams) {
    if !advanced.asynchronous_rotation_enabled {
        return;
    }
    let minimum = advanced.asynchronous_rotation_min_degrees.to_radians();
    let maximum = advanced.asynchronous_rotation_max_degrees.to_radians();
    let midpoint = (minimum + maximum) / 2.0;
    let amplitude = (maximum - minimum) / 2.0;
    let interval = advanced.slice_trigger_interval_ms as f64 / 1_000.0;
    let easing = if advanced.transform_smoothing_enabled {
        format!("*({})", transform_easing(advanced, "t"))
    } else {
        String::new()
    };
    plan.filters.push(format!(
        "rotate=angle='{midpoint:.10}+{amplitude:.10}*sin(2*PI*t/{interval:.6}){easing}':ow=iw:oh=ih:fillcolor=black"
    ));
    plan.applied_fields
        .push("advanced.asynchronous_rotation_enabled");
    plan.applied_fields
        .push("advanced.asynchronous_rotation_min_degrees");
    plan.applied_fields
        .push("advanced.asynchronous_rotation_max_degrees");
}

fn append_complex_effect_applied_fields(
    plan: &mut MediaVideoEffectPlan,
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
) {
    if video.color_space_conversion_enabled
        && video.color_space_conversion_strength_percent > f64::EPSILON
    {
        record_applied(plan, "video.color_space_conversion_enabled");
        record_applied(plan, "video.color_space_conversion_strength_percent");
    }
    if video.edge_softness_percent > 0.0 {
        record_applied(plan, "video.edge_softness_percent");
    }
    let picture_in_picture = advanced.picture_in_picture_enabled
        && advanced.picture_in_picture_opacity_percent > f64::EPSILON;
    if picture_in_picture {
        for field in [
            "advanced.picture_in_picture_enabled",
            "advanced.picture_in_picture_scale_percent",
            "advanced.picture_in_picture_opacity_percent",
            "advanced.picture_in_picture_timeline_locked",
            "advanced.overlay_offset_px",
            "advanced.slice_length_ms",
            "advanced.slice_trigger_interval_ms",
        ] {
            record_applied(plan, field);
        }
        if advanced.picture_in_picture_rotation_degrees.abs() > f64::EPSILON {
            record_applied(plan, "advanced.picture_in_picture_rotation_degrees");
        }
        if advanced.picture_in_picture_pixel_jitter_px > f64::EPSILON {
            record_applied(plan, "advanced.picture_in_picture_pixel_jitter_px");
        }
        if !advanced.picture_in_picture_timeline_locked {
            record_applied(plan, "advanced.slice_min_length_ms");
        }
    }
    if advanced.local_blur_enabled {
        for field in [
            "advanced.local_blur_enabled",
            "advanced.local_blur_region_percent",
            "advanced.local_blur_radius_px",
            "advanced.local_blur_interval_ms",
            "advanced.slice_length_ms",
        ] {
            record_applied(plan, field);
        }
    }
    if advanced.edge_fill_enabled && advanced.edge_feather_percent > 0.0 {
        record_applied(plan, "advanced.edge_feather_percent");
    }
    let smoothing_has_transform = picture_in_picture
        && (advanced.picture_in_picture_rotation_degrees.abs() > f64::EPSILON
            || advanced.picture_in_picture_pixel_jitter_px > 0.0)
        || advanced.asynchronous_rotation_enabled;
    if advanced.transform_smoothing_enabled && smoothing_has_transform {
        record_applied(plan, "advanced.transform_smoothing_enabled");
        record_applied(plan, "advanced.transform_smoothing_duration_ms");
    }
}

/// 基础串行链由 `media_engine::video_filter` 构建；这里统一记录其非中性字段，
/// 使 `applied_fields` 能覆盖最终串行链与复杂图，而不重复生成同一滤镜。
fn append_base_effect_applied_fields(plan: &mut MediaVideoEffectPlan, video: &VideoEffectParams) {
    if video.brightness_percent.abs() > f64::EPSILON {
        record_applied(plan, "video.brightness_percent");
    }
    if (video.contrast_percent - 100.0).abs() > f64::EPSILON {
        record_applied(plan, "video.contrast_percent");
    }
    if (video.saturation_percent - 100.0).abs() > f64::EPSILON {
        record_applied(plan, "video.saturation_percent");
    }
    if video.hue_rotation_degrees.abs() > f64::EPSILON {
        record_applied(plan, "video.hue_rotation_degrees");
    }
    if video.blur_radius_px > f64::EPSILON {
        record_applied(plan, "video.blur_radius_px");
    }
    if video.sharpen_percent > f64::EPSILON {
        record_applied(plan, "video.sharpen_percent");
    }
    if video.noise_percent > f64::EPSILON {
        record_applied(plan, "video.noise_percent");
    }
    if video.detail_enhancement_percent > f64::EPSILON {
        record_applied(plan, "video.detail_enhancement_percent");
    }
    if (video.pixel_scale_percent - 100.0).abs() > f64::EPSILON {
        record_applied(plan, "video.pixel_scale_percent");
    }
    if video.pixel_jitter_px > f64::EPSILON {
        record_applied(plan, "video.pixel_jitter_px");
    }
    if video.space_x_offset_px.round().abs() > f64::EPSILON {
        record_applied(plan, "video.space_x_offset_px");
    }
    if video.space_y_offset_px.round().abs() > f64::EPSILON {
        record_applied(plan, "video.space_y_offset_px");
    }
    if video.horizontal_flip_enabled {
        record_applied(plan, "video.horizontal_flip_enabled");
    }
    if video.vertical_flip_enabled {
        record_applied(plan, "video.vertical_flip_enabled");
    }
    if video.rotation_degrees.abs() > f64::EPSILON {
        record_applied(plan, "video.rotation_degrees");
    }
    if video.vignette_percent > f64::EPSILON {
        record_applied(plan, "video.vignette_percent");
    }
    let tone_active =
        video.highlights_percent.abs() > f64::EPSILON || video.shadows_percent.abs() > f64::EPSILON;
    if video.highlights_percent.abs() > f64::EPSILON {
        record_applied(plan, "video.highlights_percent");
    }
    if video.shadows_percent.abs() > f64::EPSILON {
        record_applied(plan, "video.shadows_percent");
    }
    if tone_active && video.red_channel_lock_enabled {
        record_applied(plan, "video.red_channel_lock_enabled");
    }
    if video.image_repair_enabled && video.image_repair_strength_percent > f64::EPSILON {
        record_applied(plan, "video.image_repair_enabled");
        record_applied(plan, "video.image_repair_strength_percent");
    }
}

fn record_applied(plan: &mut MediaVideoEffectPlan, field: &'static str) {
    if !plan.applied_fields.contains(&field) {
        plan.applied_fields.push(field);
    }
}

fn transform_easing(advanced: &AdvancedEffectParams, time_variable: &str) -> String {
    transform_easing_at(advanced, time_variable, 0.0)
}

fn transform_easing_at(
    advanced: &AdvancedEffectParams,
    time_variable: &str,
    start_seconds: f64,
) -> String {
    let interval = advanced.slice_trigger_interval_ms as f64 / 1_000.0;
    let length = advanced.slice_length_ms as f64 / 1_000.0;
    let duration = (advanced.transform_smoothing_duration_ms as f64 / 1_000.0)
        .min(length / 2.0)
        .max(0.001);
    let phase = if start_seconds <= f64::EPSILON {
        format!("mod({time_variable}\\,{interval:.6})")
    } else {
        format!("mod({time_variable}-{start_seconds:.6}+{interval:.6}\\,{interval:.6})")
    };
    let progress = format!("clip(min({phase}\\,{length:.6}-{phase})/{duration:.6}\\,0\\,1)");
    format!("({progress})*({progress})*(3-2*({progress}))")
}

fn abstract_face_filters(
    index: u8,
    advanced: &AdvancedEffectParams,
    alpha: f64,
    schedule: &str,
) -> Vec<String> {
    let count = f64::from(advanced.abstract_face_count);
    let x_fraction = (f64::from(index) + 1.0) / (count + 1.0);
    let y_fraction = 0.2 + 0.18 * f64::from(index % 3);
    let offset = advanced.overlay_offset_px;
    let size_fraction = advanced.abstract_face_size_percent / 100.0;
    let center_x = format!("iw*{x_fraction:.6}+{offset:.6}");
    let center_y = format!("ih*{y_fraction:.6}+{offset:.6}");
    let size = format!("iw*{size_fraction:.6}");
    vec![
        format!(
            "drawbox=x='{center_x}-{size}*0.5':y='{center_y}-{size}*0.5':w='{size}':h='{size}':color=white@{alpha:.6}:t=1:enable='{schedule}'"
        ),
        format!(
            "drawbox=x='{center_x}-{size}*0.22':y='{center_y}-{size}*0.16':w=1:h=1:color=white@{alpha:.6}:t=fill:enable='{schedule}'"
        ),
        format!(
            "drawbox=x='{center_x}+{size}*0.18':y='{center_y}-{size}*0.16':w=1:h=1:color=white@{alpha:.6}:t=fill:enable='{schedule}'"
        ),
        format!(
            "drawbox=x='{center_x}-{size}*0.2':y='{center_y}+{size}*0.18':w='max(1\\,{size}*0.4)':h=1:color=white@{alpha:.6}:t=fill:enable='{schedule}'"
        ),
    ]
}

fn append_overlay_schedule_fields(plan: &mut MediaVideoEffectPlan) {
    for field in [
        "advanced.overlay_offset_px",
        "advanced.slice_length_ms",
        "advanced.slice_trigger_interval_ms",
    ] {
        if !plan.applied_fields.contains(&field) {
            plan.applied_fields.push(field);
        }
    }
}

fn slice_enable(advanced: &AdvancedEffectParams) -> String {
    slice_enable_at(advanced, 0.0)
}

fn picture_in_picture_start_seconds(advanced: &AdvancedEffectParams) -> f64 {
    let length = advanced.slice_length_ms as f64 / 1_000.0;
    let interval = advanced.slice_trigger_interval_ms as f64 / 1_000.0;
    let latest_start = (interval - length).max(0.0);
    (advanced.slice_min_length_ms as f64 / 1_000.0).min(latest_start)
}

fn slice_enable_at(advanced: &AdvancedEffectParams, start_seconds: f64) -> String {
    let length = advanced.slice_length_ms as f64 / 1_000.0;
    let interval = advanced.slice_trigger_interval_ms as f64 / 1_000.0;
    if start_seconds <= f64::EPSILON {
        format!("lt(mod(t\\,{interval:.6})\\,{length:.6})")
    } else {
        format!(
            "between(mod(t\\,{interval:.6})\\,{start_seconds:.6}\\,{:.6})",
            start_seconds + length
        )
    }
}

fn yuv_geq(expression: &str, enable: Option<&str>) -> String {
    let mut filter = format!("geq=lum='{expression}':cb='cb(X\\,Y)':cr='cr(X\\,Y)'");
    if let Some(enable) = enable {
        filter.push_str(&format!(":enable='{enable}'"));
    }
    filter
}

fn append_frame_rate_perturbation(plan: &mut MediaVideoEffectPlan, video: &VideoEffectParams) {
    let relative_jitter = video.frame_rate_jitter_percent / 100.0;
    let amplitude_fps = video.frame_rate_perturbation_amplitude_fps;
    if relative_jitter == 0.0 && amplitude_fps == 0.0 {
        return;
    }
    let frequency_hz = video.frame_rate_perturbation_frequency_hz;
    let expression = format!(
        "if(eq(N\\,0)\\,PTS\\,PREV_OUTPTS+(PTS-PREV_INPTS)/(1+clip(({relative_jitter:.8}+{amplitude_fps:.8}*(PTS-PREV_INPTS)*TB)*sin(2*PI*{frequency_hz:.8}*T)\\,-0.5\\,0.5)))"
    );
    let strip_fps = if video.frame_rate_lock_enabled {
        ""
    } else {
        ":strip_fps=1"
    };
    plan.filters.push(format!(
        "settb=expr=AVTB,setpts=expr='{expression}'{strip_fps}"
    ));
    if relative_jitter > 0.0 {
        plan.applied_fields.push("video.frame_rate_jitter_percent");
    }
    plan.applied_fields
        .push("video.frame_rate_perturbation_frequency_hz");
    if amplitude_fps > 0.0 {
        plan.applied_fields
            .push("video.frame_rate_perturbation_amplitude_fps");
    }
    plan.requires_variable_frame_rate = true;
}

fn append_frame_rate_lock(plan: &mut MediaVideoEffectPlan, video: &VideoEffectParams) {
    if !video.frame_rate_lock_enabled {
        return;
    }
    // `source_fps` 由 fps 滤镜读取输入流平均帧率；放在时间戳扰动之后，将最终输出
    // 重新采样到源平均帧率并恢复 CFR，不需要在 IPC 重复传递同一份探测元数据。
    plan.filters
        .push("fps=fps=source_fps:round=near:eof_action=pass".to_owned());
    plan.applied_fields.push("video.frame_rate_lock_enabled");
    plan.requires_variable_frame_rate = false;
}

#[cfg(test)]
mod tests {
    use super::{build_media_video_complex_effect_plan, build_media_video_effect_plan};
    use crate::media_effect_params::{
        AdvancedEffectParams, VideoEffectParams, VISUAL_BAND_FREQUENCIES_HZ,
    };
    use std::process::{Command, Stdio};

    #[test]
    fn defaults_have_no_extra_filter() {
        let plan = build_media_video_effect_plan(
            &VideoEffectParams::default(),
            &AdvancedEffectParams::default(),
        )
        .expect("defaults are supported");

        assert!(plan.is_empty());
        assert!(!plan.requires_variable_frame_rate);
        assert!(!plan.replaces_base_dynamic_crop);
    }

    #[test]
    fn sub_percent_frame_inner_perturbation_uses_fractional_pixel_density() {
        let video = VideoEffectParams {
            frame_inner_perturbation_percent: 0.02,
            ..VideoEffectParams::default()
        };

        let plan = build_media_video_effect_plan(&video, &AdvancedEffectParams::default())
            .expect("fractional frame perturbation maps");

        assert!(plan.filter_chain().contains("geq=lum="));
        assert!(plan.filter_chain().contains("N*199+58380\\,100000)\\,20)"));
        assert!(!plan.filter_chain().contains("noise=alls=1"));
    }

    #[test]
    fn frame_inner_perturbation_shares_spatial_luma_pass() {
        let video = VideoEffectParams {
            frame_inner_perturbation_percent: 0.02,
            ..VideoEffectParams::default()
        };
        let advanced = AdvancedEffectParams {
            wave_intensity: 0.002,
            wave_level: 0.002,
            ..AdvancedEffectParams::default()
        };

        let plan = build_media_video_effect_plan(&video, &advanced)
            .expect("frame and spatial perturbations map");

        assert_eq!(
            plan.filters
                .iter()
                .filter(|filter| filter.starts_with("geq=lum="))
                .count(),
            1
        );
        assert!(plan
            .applied_fields
            .contains(&"video.frame_inner_perturbation_percent"));
        assert!(plan.applied_fields.contains(&"advanced.wave_intensity"));
    }

    #[test]
    fn sub_percent_frame_probability_uses_hundred_thousand_frame_sampling() {
        for (probability, threshold) in [(0.03, 30), (0.031, 31), (0.1, 100)] {
            let advanced = AdvancedEffectParams {
                frame_perturbation_probability_percent: probability,
                ..AdvancedEffectParams::default()
            };

            let plan = build_media_video_effect_plan(&VideoEffectParams::default(), &advanced)
                .expect("fractional probability maps");
            let chain = plan.filter_chain();
            let phase = threshold * 7_919 % 100_000;
            assert!(
                chain.contains(&format!("mod(n*37+{phase}\\,100000)")),
                "{chain}"
            );
            assert!(chain.contains(&format!("\\,{threshold})")), "{chain}");
            assert!(!chain.contains("mod(n*37\\,100)"), "{chain}");
            let expected_hits = threshold;
            let hits = (0..100_000)
                .filter(|frame| ((frame * 37 + phase) % 100_000) < expected_hits)
                .count();
            assert_eq!(hits, expected_hits);
        }
    }

    #[test]
    fn highlight_perturbation_is_one_code_value_short_pulse() {
        let advanced = AdvancedEffectParams {
            highlight_perturbation_enabled: true,
            highlight_perturbation_interval_ms: 60_000,
            ..AdvancedEffectParams::default()
        };

        let plan = build_media_video_effect_plan(&VideoEffectParams::default(), &advanced)
            .expect("highlight perturbation maps");
        let chain = plan.filter_chain();
        assert!(chain.contains("clip(val+1\\,0\\,255)"), "{chain}");
        assert!(
            chain.contains("lt(mod(t\\,60.000000)\\,0.050000)"),
            "{chain}"
        );
        assert!(!chain.contains("clip(val+4"), "{chain}");
    }

    #[test]
    fn fully_transparent_picture_in_picture_is_bypassed() {
        let advanced = AdvancedEffectParams {
            picture_in_picture_enabled: true,
            picture_in_picture_opacity_percent: 0.0,
            ..AdvancedEffectParams::default()
        };
        let plan = build_media_video_effect_plan(&VideoEffectParams::default(), &advanced)
            .expect("transparent picture-in-picture is valid");

        assert!(!plan
            .applied_fields
            .contains(&"advanced.picture_in_picture_enabled"));
        assert!(build_media_video_complex_effect_plan(
            &plan.filter_chain(),
            &VideoEffectParams::default(),
            &advanced,
        )
        .is_none());
    }

    #[test]
    fn low_opacity_picture_in_picture_uses_two_code_value_alpha() {
        let advanced = AdvancedEffectParams {
            picture_in_picture_enabled: true,
            picture_in_picture_opacity_percent: 0.1,
            ..AdvancedEffectParams::default()
        };
        let video = VideoEffectParams::default();
        let serial = build_media_video_effect_plan(&video, &advanced).expect("pip maps");
        let complex =
            build_media_video_complex_effect_plan(&serial.filter_chain(), &video, &advanced)
                .expect("pip graph");

        assert!(complex.graph.contains("colorchannelmixer=aa=0.007843"));
    }

    #[test]
    fn active_effects_generate_real_filters_and_status() {
        let video = VideoEffectParams {
            dynamic_crop_percent: 1.0,
            crop_edge_smoothing: 0.9,
            frame_rate_jitter_percent: 1.0,
            frame_rate_perturbation_frequency_hz: 0.5,
            frame_rate_perturbation_amplitude_fps: 0.5,
            frame_inner_perturbation_percent: 1.0,
            frame_inter_perturbation_percent: 5.0,
            ..VideoEffectParams::default()
        };
        let advanced = AdvancedEffectParams {
            wave_intensity: 0.4,
            wave_level: 0.2,
            channel_offset_percent: 1.0,
            space_dimension: 3,
            frequency_space_x_offset_px: 2.0,
            frequency_space_y_offset_px: -2.0,
            frame_perturbation_probability_percent: 5.0,
            random_graphic_enabled: true,
            random_graphic_count: 3,
            random_graphic_opacity_percent: 10.0,
            abstract_face_count: 2,
            abstract_face_opacity_percent: 10.0,
            ..AdvancedEffectParams::default()
        };

        let plan = build_media_video_effect_plan(&video, &advanced).expect("effects supported");

        assert!(plan.requires_variable_frame_rate);
        assert!(plan.replaces_base_dynamic_crop);
        for filter in ["scale=", "noise=", "tblend=", "geq=", "setpts="] {
            assert!(plan.filter_chain().contains(filter), "{filter}");
        }
        for field in [
            "video.crop_edge_smoothing",
            "video.frame_rate_jitter_percent",
            "advanced.wave_intensity",
            "advanced.random_graphic_opacity_percent",
            "advanced.abstract_face_count",
            "advanced.slice_trigger_interval_ms",
        ] {
            assert!(plan.applied_fields.contains(&field), "{field}");
        }
    }

    #[test]
    fn channel_offset_uses_one_code_value_lut_instead_of_spatial_shift() {
        let advanced = AdvancedEffectParams {
            channel_offset_percent: 0.02,
            ..AdvancedEffectParams::default()
        };

        let plan = build_media_video_effect_plan(&VideoEffectParams::default(), &advanced)
            .expect("channel offset maps");
        assert_eq!(
            plan.filters,
            ["lutrgb=r='clip(val+1\\,0\\,255)':g='val':b='clip(val-1\\,0\\,255)'"]
        );
        assert!(!plan.filter_chain().contains("rgbashift="));
        assert!(!plan.filter_chain().contains("geq="));
        assert!(plan
            .applied_fields
            .contains(&"advanced.channel_offset_percent"));
    }

    #[test]
    fn formerly_planned_fields_generate_filters_and_runtime_status() {
        let video = VideoEffectParams {
            edge_softness_percent: 40.0,
            frame_rate_lock_enabled: true,
            ..VideoEffectParams::default()
        };
        let mut advanced = AdvancedEffectParams {
            target_frequency_hz: Some(500.0),
            core_frequency_hz: Some(250.0),
            dynamic_eq_threshold: 11.0,
            slice_min_length_ms: 1_000,
            picture_in_picture_enabled: true,
            picture_in_picture_timeline_locked: false,
            picture_in_picture_rotation_degrees: 2.0,
            picture_in_picture_pixel_jitter_px: 1.5,
            edge_fill_enabled: true,
            edge_feather_percent: 50.0,
            transform_smoothing_enabled: true,
            transform_smoothing_duration_ms: 500,
            ..AdvancedEffectParams::default()
        };
        advanced.band_weights.insert(65, 1.1);

        let plan = build_media_video_effect_plan(&video, &advanced).expect("all fields mapped");
        assert!(plan.filter_chain().contains("geq="));
        assert!(plan.filter_chain().contains("fps=fps=source_fps"));
        assert!(!plan.requires_variable_frame_rate);
        for field in [
            "video.edge_softness_percent",
            "video.frame_rate_lock_enabled",
            "advanced.band_weights",
            "advanced.target_frequency_hz",
            "advanced.core_frequency_hz",
            "advanced.dynamic_eq_threshold",
            "advanced.slice_min_length_ms",
            "advanced.picture_in_picture_timeline_locked",
            "advanced.picture_in_picture_opacity_percent",
            "advanced.edge_feather_percent",
            "advanced.transform_smoothing_enabled",
            "advanced.transform_smoothing_duration_ms",
        ] {
            assert!(plan.applied_fields.contains(&field), "{field}");
        }

        let complex =
            build_media_video_complex_effect_plan(&plan.filter_chain(), &video, &advanced)
                .expect("edge and unlocked picture-in-picture require a graph");
        for fragment in [
            "fillborders=",
            "blend=all_opacity=0.500000",
            "blend=all_opacity=0.400000",
            "gblur=sigma=",
            "colorchannelmixer=aa=1.000000",
            "setpts=PTS-STARTPTS+1.000000/TB",
            "enable='between(mod(t\\,15.000000)\\,1.000000\\,6.000000)'",
            "eof_action=pass",
        ] {
            assert!(complex.graph.contains(fragment), "{fragment}");
        }
        assert!(!complex.graph.contains("format=gray,geq="));
        assert!(!complex.graph.contains("maskedmerge"));
        assert!(!complex.graph.contains("trim=duration="));
        assert!(complex.graph.contains("*(3-2*("), "smoothstep gate");
    }

    #[test]
    fn low_perception_spatial_fields_share_one_luma_pass() {
        let mut advanced = AdvancedEffectParams {
            target_frequency_hz: Some(4_000.0),
            core_frequency_hz: Some(2_000.0),
            dynamic_eq_threshold: 0.05,
            wave_intensity: 0.002,
            wave_level: 0.002,
            wave_grain_count: 9,
            space_dimension: 3,
            frequency_space_x_offset_px: 0.1,
            frequency_space_y_offset_px: -0.1,
            ..AdvancedEffectParams::default()
        };
        for (index, weight) in advanced.band_weights.values_mut().enumerate() {
            *weight = if index % 2 == 0 { 0.999 } else { 1.001 };
        }

        let plan = build_media_video_effect_plan(&VideoEffectParams::default(), &advanced)
            .expect("spatial fields map");
        assert_eq!(
            plan.filters
                .iter()
                .filter(|filter| filter.starts_with("geq=lum="))
                .count(),
            1,
            "band and wave modulation must not traverse every pixel twice"
        );
        for field in [
            "advanced.band_weights",
            "advanced.target_frequency_hz",
            "advanced.core_frequency_hz",
            "advanced.dynamic_eq_threshold",
            "advanced.wave_intensity",
            "advanced.wave_level",
            "advanced.wave_grain_count",
            "advanced.space_dimension",
            "advanced.frequency_space_x_offset_px",
            "advanced.frequency_space_y_offset_px",
        ] {
            assert!(plan.applied_fields.contains(&field), "{field}");
        }
    }

    #[test]
    fn low_opacity_graphics_share_one_pass_with_two_code_value_alpha() {
        let advanced = AdvancedEffectParams {
            random_graphic_enabled: true,
            random_graphic_count: 1,
            random_graphic_opacity_percent: 0.1,
            abstract_face_count: 1,
            abstract_face_opacity_percent: 0.05,
            ..AdvancedEffectParams::default()
        };

        let plan = build_media_video_effect_plan(&VideoEffectParams::default(), &advanced)
            .expect("graphics map");
        let luma_graphics = plan
            .filters
            .iter()
            .filter(|filter| filter.starts_with("geq=lum="))
            .collect::<Vec<_>>();
        assert!(
            luma_graphics.is_empty(),
            "small overlays must not scan every pixel with geq"
        );
        let drawboxes = plan
            .filters
            .iter()
            .filter(|filter| filter.starts_with("drawbox="))
            .collect::<Vec<_>>();
        assert!(drawboxes.len() >= 2);
        assert!(
            drawboxes
                .iter()
                .filter(|filter| filter.contains("white@0.007843"))
                .count()
                >= 2,
            "both overlays need at least 2/255 effective alpha"
        );
    }

    #[test]
    fn automatic_low_perception_snapshot_records_every_executed_field() {
        let video = VideoEffectParams {
            brightness_percent: 0.2,
            saturation_percent: 100.2,
            blur_radius_px: 0.02,
            contrast_percent: 99.8,
            hue_rotation_degrees: 0.1,
            sharpen_percent: 0.2,
            noise_percent: 0.05,
            detail_enhancement_percent: 0.2,
            crop_edge_smoothing: 0.8,
            frame_rate_jitter_percent: 0.02,
            frame_rate_perturbation_frequency_hz: 0.1,
            frame_rate_perturbation_amplitude_fps: 0.02,
            pixel_scale_percent: 100.1,
            pixel_jitter_px: 0.05,
            dynamic_crop_percent: 0.05,
            frame_inner_perturbation_percent: 0.02,
            frame_inter_perturbation_percent: 0.05,
            space_x_offset_px: 0.5,
            space_y_offset_px: -0.5,
            color_space_conversion_strength_percent: 0.2,
            color_space_conversion_enabled: true,
            horizontal_flip_enabled: false,
            vertical_flip_enabled: false,
            rotation_degrees: 0.02,
            vignette_percent: 0.1,
            highlights_percent: 0.1,
            shadows_percent: -0.1,
            red_channel_lock_enabled: true,
            edge_softness_percent: 0.1,
            image_repair_enabled: true,
            image_repair_strength_percent: 0.1,
            frame_rate_lock_enabled: true,
        };
        let mut advanced = AdvancedEffectParams {
            target_frequency_hz: Some(500.0),
            core_frequency_hz: Some(250.0),
            wave_intensity: 0.002,
            wave_level: 0.002,
            wave_grain_count: 9,
            dynamic_eq_threshold: 0.05,
            channel_offset_percent: 0.02,
            space_dimension: 3,
            frequency_space_x_offset_px: 0.1,
            frequency_space_y_offset_px: -0.1,
            frame_perturbation_probability_percent: 0.05,
            random_graphic_opacity_percent: 0.1,
            random_graphic_size_px: 1.5,
            abstract_face_count: 1,
            abstract_face_size_percent: 1.2,
            abstract_face_opacity_percent: 0.05,
            overlay_offset_px: 0.1,
            slice_length_ms: 500,
            slice_min_length_ms: 1_000,
            slice_trigger_interval_ms: 30_000,
            random_graphic_enabled: true,
            random_graphic_count: 1,
            picture_in_picture_enabled: true,
            picture_in_picture_scale_percent: 10.0,
            picture_in_picture_opacity_percent: 0.5,
            picture_in_picture_rotation_degrees: 0.05,
            picture_in_picture_pixel_jitter_px: 0.05,
            picture_in_picture_timeline_locked: false,
            local_blur_enabled: true,
            local_blur_region_percent: 5.0,
            local_blur_radius_px: 0.1,
            local_blur_interval_ms: 30_000,
            edge_fill_enabled: true,
            edge_feather_percent: 0.2,
            transform_smoothing_enabled: true,
            transform_smoothing_duration_ms: 100,
            highlight_perturbation_enabled: true,
            highlight_perturbation_interval_ms: 60_000,
            asynchronous_rotation_enabled: true,
            asynchronous_rotation_min_degrees: -0.02,
            asynchronous_rotation_max_degrees: 0.02,
            ..AdvancedEffectParams::default()
        };
        for (index, weight) in advanced.band_weights.values_mut().enumerate() {
            *weight = if index % 2 == 0 { 0.999 } else { 1.001 };
        }

        video.validate().expect("video snapshot is valid");
        advanced.validate().expect("advanced snapshot is valid");
        let plan = build_media_video_effect_plan(&video, &advanced).expect("snapshot maps");

        for field in [
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
            "advanced.band_weights",
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
            "advanced.random_graphic_enabled",
            "advanced.random_graphic_count",
            "advanced.random_graphic_opacity_percent",
            "advanced.random_graphic_size_px",
            "advanced.abstract_face_count",
            "advanced.abstract_face_size_percent",
            "advanced.abstract_face_opacity_percent",
            "advanced.overlay_offset_px",
            "advanced.slice_length_ms",
            "advanced.slice_min_length_ms",
            "advanced.slice_trigger_interval_ms",
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
        ] {
            assert!(plan.applied_fields.contains(&field), "{field}");
        }
        assert!(!plan
            .applied_fields
            .contains(&"video.horizontal_flip_enabled"));
        assert!(!plan.applied_fields.contains(&"video.vertical_flip_enabled"));

        let complex =
            build_media_video_complex_effect_plan(&plan.filter_chain(), &video, &advanced)
                .expect("automatic snapshot needs a complex graph");
        assert!(complex.graph.contains("colorchannelmixer=aa=0.007843"));

        if let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") {
            let baseline = render_test_frames_with_duration(&ffmpeg, None, 3, 25);
            let effected = render_test_frames_with_duration(&ffmpeg, Some(&complex.graph), 3, 25);
            assert!(baseline.status.success(), "{:?}", baseline.status);
            assert!(
                effected.status.success(),
                "{}",
                String::from_utf8_lossy(&effected.stderr)
            );
            assert_eq!(baseline.stdout.len(), effected.stdout.len());
            assert_ne!(baseline.stdout, effected.stdout);
        }
    }

    #[test]
    fn every_fixed_visual_band_maps_to_a_distinct_logarithmic_spatial_carrier() {
        let mut chains = Vec::new();
        for frequency_hz in VISUAL_BAND_FREQUENCIES_HZ {
            let mut advanced = AdvancedEffectParams::default();
            advanced.band_weights.insert(frequency_hz, 1.5);
            let plan = build_media_video_effect_plan(&VideoEffectParams::default(), &advanced)
                .expect("fixed visual band mapped");
            assert!(plan.applied_fields.contains(&"advanced.band_weights"));
            chains.push(plan.filter_chain());
        }
        chains.dedup();
        assert_eq!(chains.len(), VISUAL_BAND_FREQUENCIES_HZ.len());
        assert!(chains
            .first()
            .is_some_and(|chain| chain.contains("1.00000000")));
        assert!(chains
            .last()
            .is_some_and(|chain| chain.contains("32.00000000")));
    }

    #[test]
    fn branching_effects_build_one_labeled_complex_graph() {
        let video = VideoEffectParams {
            color_space_conversion_enabled: true,
            color_space_conversion_strength_percent: 35.0,
            ..VideoEffectParams::default()
        };
        let advanced = AdvancedEffectParams {
            local_blur_enabled: true,
            picture_in_picture_enabled: true,
            picture_in_picture_rotation_degrees: 2.0,
            ..AdvancedEffectParams::default()
        };

        let plan = build_media_video_complex_effect_plan("null", &video, &advanced)
            .expect("branching effects should require a graph");
        for fragment in [
            "[0:v:0]null",
            "colorspace=iall=bt601-6-625:all=bt709",
            "gblur=sigma=",
            "blend=all_expr=",
            "overlay=x=",
            "[vout]",
        ] {
            assert!(plan.graph.contains(fragment), "{fragment}");
        }
    }

    #[test]
    fn packaged_ffmpeg_accepts_branching_effect_graph() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let video = VideoEffectParams {
            color_space_conversion_enabled: true,
            color_space_conversion_strength_percent: 35.0,
            ..VideoEffectParams::default()
        };
        let advanced = AdvancedEffectParams {
            local_blur_enabled: true,
            picture_in_picture_enabled: true,
            picture_in_picture_rotation_degrees: 2.0,
            ..AdvancedEffectParams::default()
        };
        let plan = build_media_video_complex_effect_plan("null", &video, &advanced)
            .expect("branching graph");

        let output = Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=duration=1:size=64x64:rate=10",
                "-filter_complex",
                &plan.graph,
                "-map",
                "[vout]",
                "-frames:v",
                "5",
                "-pix_fmt",
                "gray",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .output()
            .expect("run packaged ffmpeg");

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty());
    }

    #[test]
    fn packaged_ffmpeg_accepts_chain_and_changes_pixels_when_configured() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let video = VideoEffectParams {
            dynamic_crop_percent: 1.0,
            crop_edge_smoothing: 0.9,
            frame_rate_jitter_percent: 1.0,
            frame_rate_perturbation_frequency_hz: 0.5,
            frame_rate_perturbation_amplitude_fps: 0.5,
            frame_inner_perturbation_percent: 1.0,
            frame_inter_perturbation_percent: 5.0,
            ..VideoEffectParams::default()
        };
        let advanced = AdvancedEffectParams {
            wave_intensity: 0.4,
            wave_level: 0.2,
            channel_offset_percent: 1.0,
            frame_perturbation_probability_percent: 5.0,
            random_graphic_enabled: true,
            random_graphic_opacity_percent: 10.0,
            abstract_face_count: 1,
            abstract_face_opacity_percent: 10.0,
            ..AdvancedEffectParams::default()
        };
        let plan = build_media_video_effect_plan(&video, &advanced).expect("effects supported");

        let baseline = render_test_frames(&ffmpeg, None);
        let effected = render_test_frames(&ffmpeg, Some(&plan.filter_chain()));
        assert!(baseline.status.success(), "{:?}", baseline.status);
        assert!(
            effected.status.success(),
            "{}",
            String::from_utf8_lossy(&effected.stderr)
        );
        assert_eq!(baseline.stdout.len(), effected.stdout.len());
        assert_ne!(baseline.stdout, effected.stdout);
    }

    #[test]
    fn packaged_ffmpeg_accepts_all_formerly_planned_video_fields_and_changes_pixels() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let video = VideoEffectParams {
            edge_softness_percent: 40.0,
            frame_rate_lock_enabled: true,
            ..VideoEffectParams::default()
        };
        let mut advanced = AdvancedEffectParams {
            target_frequency_hz: Some(500.0),
            core_frequency_hz: Some(250.0),
            dynamic_eq_threshold: 8.0,
            slice_min_length_ms: 1_000,
            picture_in_picture_enabled: true,
            picture_in_picture_timeline_locked: false,
            picture_in_picture_rotation_degrees: 2.0,
            picture_in_picture_pixel_jitter_px: 1.5,
            edge_fill_enabled: true,
            edge_feather_percent: 50.0,
            transform_smoothing_enabled: true,
            transform_smoothing_duration_ms: 400,
            asynchronous_rotation_enabled: true,
            ..AdvancedEffectParams::default()
        };
        advanced.band_weights.insert(65, 1.5);
        advanced.band_weights.insert(20_000, 0.5);
        let serial = build_media_video_effect_plan(&video, &advanced).expect("all fields mapped");
        let complex =
            build_media_video_complex_effect_plan(&serial.filter_chain(), &video, &advanced)
                .expect("complex graph");

        let baseline = render_test_frames_with_duration(&ffmpeg, None, 3, 25);
        let effected = render_test_frames_with_duration(&ffmpeg, Some(&complex.graph), 3, 25);
        assert!(baseline.status.success(), "{:?}", baseline.status);
        assert!(
            effected.status.success(),
            "{}",
            String::from_utf8_lossy(&effected.stderr)
        );
        assert_eq!(baseline.stdout.len(), effected.stdout.len());
        assert_ne!(baseline.stdout, effected.stdout);
    }

    fn render_test_frames(ffmpeg: &std::ffi::OsStr, filter: Option<&str>) -> std::process::Output {
        let mut command = Command::new(ffmpeg);
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=duration=1:size=64x64:rate=10",
        ]);
        if let Some(filter) = filter {
            command.args(["-vf", filter]);
        }
        command
            .args([
                "-frames:v",
                "5",
                "-fps_mode",
                "vfr",
                "-pix_fmt",
                "gray",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .output()
            .expect("run packaged ffmpeg")
    }

    fn render_test_frames_with_duration(
        ffmpeg: &std::ffi::OsStr,
        graph: Option<&str>,
        duration_seconds: u8,
        frame_count: u8,
    ) -> std::process::Output {
        let mut command = Command::new(ffmpeg);
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=duration={duration_seconds}:size=64x64:rate=10"),
        ]);
        if let Some(graph) = graph {
            command.args(["-filter_complex", graph, "-map", "[vout]"]);
        }
        command
            .args([
                "-frames:v",
                &frame_count.to_string(),
                "-pix_fmt",
                "gray",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .output()
            .expect("run packaged ffmpeg")
    }
}
