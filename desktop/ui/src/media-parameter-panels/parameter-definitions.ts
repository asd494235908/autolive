import type {
  MediaParameterPath,
  MediaParameterSection,
  MediaParameterStatus,
} from './media-parameter-types';

export const VISUAL_BAND_FREQUENCIES_HZ = [
  65, 92, 131, 188, 267, 381, 544, 777, 1110, 1585, 2263, 20_000,
] as const;

export const MEDIA_PARAMETER_STATUS_LABELS: Record<MediaParameterStatus, string> = {
  implemented: '已接入',
  planned: '正式需求待实现',
  pending_confirmation: '待确认',
};

export interface ParameterOption {
  key: string;
  label: string;
  value: string | number | null;
}

interface BaseParameterDefinition {
  section: MediaParameterSection;
  field: string;
  group: string;
  label: string;
  status: MediaParameterStatus;
  description?: string;
}

export interface NumericParameterDefinition extends BaseParameterDefinition {
  kind: 'number' | 'optional-number' | 'readonly-number';
  min: number;
  max: number;
  step: number;
  unit: string;
  enableValue?: number;
}

export interface SelectParameterDefinition extends BaseParameterDefinition {
  kind: 'select';
  options: readonly ParameterOption[];
}

export interface BooleanParameterDefinition extends BaseParameterDefinition {
  kind: 'boolean';
}

export interface TextParameterDefinition extends BaseParameterDefinition {
  kind: 'text';
  maxLength: number;
}

export interface BandWeightsParameterDefinition extends BaseParameterDefinition {
  kind: 'band-weights';
  min: number;
  max: number;
  step: number;
  unit: string;
}

export type MediaParameterDefinition =
  | NumericParameterDefinition
  | SelectParameterDefinition
  | BooleanParameterDefinition
  | TextParameterDefinition
  | BandWeightsParameterDefinition;

type NumberDefinitionInput = Omit<NumericParameterDefinition, 'kind'> & {
  kind?: NumericParameterDefinition['kind'];
};

function numberDefinition(input: NumberDefinitionInput): NumericParameterDefinition {
  return { kind: 'number', ...input };
}

function booleanDefinition(
  input: Omit<BooleanParameterDefinition, 'kind'>,
): BooleanParameterDefinition {
  return { kind: 'boolean', ...input };
}

const implementedVideoDescription = '已进入 FFmpeg 视频滤镜链。';
const implementedAdvancedDescription = '已进入 FFmpeg 高级视觉滤镜链。';

export const VIDEO_PARAMETER_DEFINITIONS: readonly MediaParameterDefinition[] = [
  numberDefinition({ section: 'video', field: 'brightness_percent', group: '色彩', label: '亮度', status: 'implemented', min: -100, max: 100, step: 0.1, unit: '%', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'contrast_percent', group: '色彩', label: '对比度', status: 'implemented', min: 0, max: 200, step: 0.1, unit: '%', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'saturation_percent', group: '色彩', label: '饱和度', status: 'implemented', min: 0, max: 200, step: 0.1, unit: '%', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'hue_rotation_degrees', group: '色彩', label: '色相旋转', status: 'implemented', min: -180, max: 180, step: 0.1, unit: '°', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'highlights_percent', group: '色彩', label: '高光', status: 'implemented', min: -100, max: 100, step: 0.1, unit: '%', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'shadows_percent', group: '色彩', label: '阴影', status: 'implemented', min: -100, max: 100, step: 0.1, unit: '%', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'vignette_percent', group: '色彩', label: '暗角', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: implementedVideoDescription }),
  booleanDefinition({ section: 'video', field: 'red_channel_lock_enabled', group: '色彩', label: '红色通道保护', status: 'implemented', description: '已作为高光/阴影和色彩转换的红通道保护开关接入。' }),
  booleanDefinition({ section: 'video', field: 'color_space_conversion_enabled', group: '色彩', label: '色域转换', status: 'implemented', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'color_space_conversion_strength_percent', group: '色彩', label: '色域转换强度', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: implementedVideoDescription }),

  numberDefinition({ section: 'video', field: 'blur_radius_px', group: '清晰度', label: '模糊半径', status: 'implemented', min: 0, max: 8, step: 0.01, unit: 'px', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'sharpen_percent', group: '清晰度', label: '锐化', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'noise_percent', group: '清晰度', label: '噪点', status: 'implemented', min: 0, max: 8, step: 0.01, unit: '%', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'detail_enhancement_percent', group: '清晰度', label: '细节增强', status: 'implemented', min: 0, max: 50, step: 0.1, unit: '%', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'crop_edge_smoothing', group: '清晰度', label: '裁剪边缘平滑', status: 'implemented', min: 0, max: 1, step: 0.01, unit: '归一化', description: '已映射到动态裁剪后的缩放插值等级。' }),
  numberDefinition({ section: 'video', field: 'edge_softness_percent', group: '清晰度', label: '画面边缘柔化', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: '已通过边缘掩码、模糊和回合成进入 FFmpeg 视频链。' }),
  booleanDefinition({ section: 'video', field: 'image_repair_enabled', group: '清晰度', label: '图像修复', status: 'implemented', description: '已接入非局部均值去噪与温和细节恢复。' }),
  numberDefinition({ section: 'video', field: 'image_repair_strength_percent', group: '清晰度', label: '图像修复强度', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: implementedVideoDescription }),

  numberDefinition({ section: 'video', field: 'pixel_scale_percent', group: '空间变换', label: '像素级缩放', status: 'implemented', min: 95, max: 105, step: 0.01, unit: '%', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'pixel_jitter_px', group: '空间变换', label: '像素级扰动', status: 'implemented', min: 0, max: 2, step: 0.01, unit: 'px', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'dynamic_crop_percent', group: '空间变换', label: '动态裁剪', status: 'implemented', min: 0, max: 4, step: 0.01, unit: '%/边', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'space_x_offset_px', group: '空间变换', label: '水平空间偏移', status: 'implemented', min: -4, max: 4, step: 0.01, unit: 'px', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'space_y_offset_px', group: '空间变换', label: '垂直空间偏移', status: 'implemented', min: -4, max: 4, step: 0.01, unit: 'px', description: implementedVideoDescription }),
  booleanDefinition({ section: 'video', field: 'horizontal_flip_enabled', group: '空间变换', label: '水平翻转', status: 'implemented', description: implementedVideoDescription }),
  booleanDefinition({ section: 'video', field: 'vertical_flip_enabled', group: '空间变换', label: '垂直翻转', status: 'implemented', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'rotation_degrees', group: '空间变换', label: '画面旋转', status: 'implemented', min: -180, max: 180, step: 0.1, unit: '°', description: implementedVideoDescription }),

  numberDefinition({ section: 'video', field: 'frame_rate_jitter_percent', group: '时间扰动', label: '帧率微扰', status: 'implemented', min: 0, max: 2, step: 0.01, unit: '%', description: '已映射为可变帧率时间戳。' }),
  numberDefinition({ section: 'video', field: 'frame_rate_perturbation_frequency_hz', group: '时间扰动', label: '帧率微扰频率', status: 'implemented', min: 0.01, max: 2, step: 0.01, unit: 'Hz', description: '已映射为可变帧率时间戳。' }),
  numberDefinition({ section: 'video', field: 'frame_rate_perturbation_amplitude_fps', group: '时间扰动', label: '帧率微扰幅度', status: 'implemented', min: 0, max: 2, step: 0.01, unit: 'fps', description: '已映射为可变帧率时间戳。' }),
  numberDefinition({ section: 'video', field: 'frame_inner_perturbation_percent', group: '时间扰动', label: '帧内微扰', status: 'implemented', min: 0, max: 2, step: 0.01, unit: '%', description: implementedVideoDescription }),
  numberDefinition({ section: 'video', field: 'frame_inter_perturbation_percent', group: '时间扰动', label: '帧间微扰', status: 'implemented', min: 0, max: 20, step: 0.1, unit: '%', description: implementedVideoDescription }),
  booleanDefinition({ section: 'video', field: 'frame_rate_lock_enabled', group: '时间扰动', label: '帧率动态锁定', status: 'implemented', description: '已在时间戳扰动后按源平均帧率重新采样为 CFR。' }),
];

const implementedAudioDescription = '已进入当前普通声音处理链。';

export const AUDIO_PARAMETER_DEFINITIONS: readonly MediaParameterDefinition[] = [
  {
    section: 'audio', field: 'natural_voice_mode', group: '模式与调度', label: '自然真人模式', kind: 'select', status: 'implemented', description: 'original 保持源声；natural_dynamic 使用变化周期驱动的缓慢响度包络。',
    options: [
      { key: 'original', label: '保持原声', value: 'original' },
      { key: 'natural_dynamic', label: '自然动态', value: 'natural_dynamic' },
    ],
  },
  numberDefinition({ section: 'audio', field: 'random_change_period_ms', group: '模式与调度', label: '参数变化周期', status: 'implemented', min: 500, max: 60_000, step: 100, unit: 'ms', description: '已接入前端周期调度，不作为 FFmpeg 滤镜参数。' }),
  { section: 'audio', field: 'voice_library_id', group: '模式与调度', label: '音色库 ID', kind: 'text', maxLength: 128, status: 'implemented', description: '本地资源 ID 稳定映射到轻量音色 EQ 预设；不下载模型。' },

  numberDefinition({ section: 'audio', field: 'pitch_shift_semitones', group: '变调与变速', label: '高质量音高', status: 'implemented', min: -2, max: 2, step: 0.01, unit: '半音', description: '已接入 Signalsmith Stretch 混音后总线。' }),
  numberDefinition({ section: 'audio', field: 'formant_shift_percent', group: '变调与变速', label: '共振峰偏移', status: 'implemented', min: -5, max: 5, step: 0.01, unit: '%', description: '已接入 Signalsmith Stretch 混音后总线。' }),
  numberDefinition({ section: 'audio', field: 'playback_speed', group: '变调与变速', label: '播放速度', status: 'implemented', min: 0.5, max: 2, step: 0.01, unit: '倍', description: '已接入 FFmpeg atempo；由普通声音预设与周期随机化更新。' }),
  numberDefinition({ section: 'audio', field: 'vibrato_frequency_hz', group: '变调与变速', label: '颤音频率', status: 'implemented', min: 3, max: 8, step: 0.01, unit: 'Hz', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'vibrato_depth_percent', group: '变调与变速', label: '颤音深度', status: 'implemented', min: 0, max: 3, step: 0.01, unit: '%', description: implementedAudioDescription }),

  numberDefinition({ section: 'audio', field: 'input_gain_db', group: '增益与均衡', label: '输入增益', status: 'implemented', min: -6, max: 6, step: 0.01, unit: 'dB', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'output_gain_db', group: '增益与均衡', label: '输出增益', status: 'implemented', min: -6, max: 6, step: 0.01, unit: 'dB', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'loudness_adjustment_db', group: '增益与均衡', label: '响度调整', status: 'implemented', min: -6, max: 6, step: 0.01, unit: 'dB', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'low_eq_db', group: '增益与均衡', label: '低频 EQ', status: 'implemented', min: -12, max: 12, step: 0.01, unit: 'dB', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'mid_eq_db', group: '增益与均衡', label: '中频 EQ', status: 'implemented', min: -12, max: 12, step: 0.01, unit: 'dB', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'high_eq_db', group: '增益与均衡', label: '高频 EQ', status: 'implemented', min: -12, max: 12, step: 0.01, unit: 'dB', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'filter_q', group: '增益与均衡', label: '滤波 Q', status: 'implemented', min: 0.3, max: 10, step: 0.01, unit: '无量纲', description: implementedAudioDescription }),

  numberDefinition({ section: 'audio', field: 'environment_noise_percent', group: '噪声与空间', label: '环境噪声混入', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'environment_noise_dbfs', group: '噪声与空间', label: '环境噪声电平', status: 'implemented', min: -60, max: -20, step: 0.1, unit: 'dBFS', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'noise_reduction_percent', group: '噪声与空间', label: '降噪', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'phase_perturbation_percent', group: '噪声与空间', label: '相位扰动', status: 'implemented', min: -20, max: 20, step: 0.1, unit: '%', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'reverb_wet_percent', group: '噪声与空间', label: '轻混响湿声', status: 'implemented', min: 0, max: 20, step: 0.1, unit: '%', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'fade_in_ms', group: '噪声与空间', label: '淡入', status: 'implemented', min: 0, max: 10_000, step: 10, unit: 'ms', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'fade_out_ms', group: '噪声与空间', label: '淡出', status: 'implemented', min: 0, max: 10_000, step: 10, unit: 'ms', description: '有限流/编码输出已接入；实时循环不会反向缓存整轮音频。' }),

  numberDefinition({ section: 'audio', field: 'spectral_perturbation_percent', group: '频域效果', label: '频谱扰动', status: 'implemented', min: 0, max: 10, step: 0.1, unit: '%', description: '已接入 FFmpeg afftfilt 幅度频谱扰动。' }),
  numberDefinition({ section: 'audio', field: 'mfcc_shift_percent', group: '特征与信噪比', label: 'MFCC 偏移', status: 'implemented', min: -20, max: 20, step: 0.1, unit: '%', description: '已接入 realFFT/rustdct 的 STFT-MFCC 重建 PCM 链。' }),
  numberDefinition({ section: 'audio', field: 'ambient_sound_mix_percent', group: '特征与信噪比', label: '环境声素材混合', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: '已接入本地环境声循环输入与 amix；用户素材可选覆盖，空路径由本地媒体引擎解析内置环境声。' }),
  numberDefinition({ section: 'audio', field: 'dry_wet_percent', group: '特征与信噪比', label: '干湿比', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: '已接入独立短反射湿声支路并与干声混合。' }),
  numberDefinition({ section: 'audio', field: 'mfcc_dimensions', group: '特征与信噪比', label: 'MFCC 维度', status: 'implemented', min: 1, max: 40, step: 1, unit: '阶', description: '控制实际 MFCC 分析、偏移与诊断返回维数。' }),
  numberDefinition({ section: 'audio', field: 'snr_variation_db', group: '特征与信噪比', label: 'SNR 浮动', status: 'implemented', min: -6, max: 6, step: 0.1, unit: 'dB', description: '已接入滚动 RMS 目标噪声注入的确定性变化。' }),
  numberDefinition({ section: 'audio', field: 'spectrum_blind_spot_percent', group: '特征与信噪比', label: '频谱盲区宽度', status: 'implemented', min: 0, max: 5, step: 0.01, unit: '%', description: '已映射为 8kHz 中心、按百分比换算带宽的 bandreject。' }),
  numberDefinition({ section: 'audio', field: 'snr_target_db', group: '特征与信噪比', label: '目标信噪比', kind: 'optional-number', status: 'implemented', min: 0, max: 60, step: 0.1, unit: 'dB', enableValue: 30, description: '已接入滚动 PCM RMS 目标信噪比处理；空值自动跟随源基线。' }),
  numberDefinition({ section: 'audio', field: 'current_formant_hz', group: '特征与信噪比', label: '当前共振峰测量值', kind: 'readonly-number', status: 'implemented', min: 20, max: 10_000, step: 1, unit: 'Hz', description: '已由 LPC 实时分析 F1，并保持为只读诊断值。' }),
  booleanDefinition({ section: 'audio', field: 'high_frequency_perturbation_enabled', group: '频域效果', label: '高频音频扰动', status: 'implemented', description: '已接入 6kHz 以上频段的周期性 afftfilt 扰动。' }),
  numberDefinition({ section: 'audio', field: 'high_frequency_perturbation_interval_ms', group: '频域效果', label: '高频扰动间隔', status: 'implemented', min: 500, max: 60_000, step: 100, unit: 'ms', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'high_frequency_perturbation_strength_percent', group: '频域效果', label: '高频扰动强度', status: 'implemented', min: 0, max: 20, step: 0.1, unit: '%', description: implementedAudioDescription }),
  numberDefinition({ section: 'audio', field: 'high_frequency_perturbation_level_db', group: '频域效果', label: '高频扰动电平', status: 'implemented', min: -60, max: 0, step: 0.1, unit: 'dB', description: implementedAudioDescription }),

  {
    section: 'audio', field: 'sample_rate_hz', group: '输出', label: '目标采样率', kind: 'select', status: 'implemented', description: '已接入 FFmpeg 采样率归一化。',
    options: [
      { key: 'source', label: '跟随源素材', value: null },
      { key: '44100', label: '44100 Hz', value: 44_100 },
      { key: '48000', label: '48000 Hz', value: 48_000 },
    ],
  },
  numberDefinition({ section: 'audio', field: 'output_bitrate_kbps', group: '输出', label: '输出音频码率', status: 'implemented', min: 64, max: 320, step: 1, unit: 'kbps', description: '仅在存在 AAC 编码输出的媒体链中生效。' }),
];

export const ADVANCED_PARAMETER_DEFINITIONS: readonly MediaParameterDefinition[] = [
  { section: 'advanced', field: 'band_weights', group: '视觉频段', label: '固定视觉频段权重', kind: 'band-weights', status: 'implemented', min: 0.5, max: 1.5, step: 0.01, unit: '倍', description: '65–20000Hz 是视频空间频段，按对数映射到 1–32 个画面周期。' },
  numberDefinition({ section: 'advanced', field: 'target_frequency_hz', group: '视觉频段', label: '目标频率', kind: 'optional-number', status: 'implemented', min: 65, max: 20_000, step: 1, unit: 'Hz', enableValue: 500, description: '选择视频空间调制中心频段；空值关闭频段调制。' }),
  numberDefinition({ section: 'advanced', field: 'core_frequency_hz', group: '视觉频段', label: '核心频率', kind: 'optional-number', status: 'implemented', min: 65, max: 20_000, step: 1, unit: 'Hz', enableValue: 500, description: '控制视频空间载波相位；空值跟随目标频率。' }),
  numberDefinition({ section: 'advanced', field: 'wave_intensity', group: '视觉频段', label: '波频强度', status: 'implemented', min: 0, max: 1, step: 0.01, unit: '归一化', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'wave_level', group: '视觉频段', label: '波频电平', status: 'implemented', min: 0, max: 1, step: 0.01, unit: '归一化', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'wave_grain_count', group: '视觉频段', label: '波频颗粒数', status: 'implemented', min: 1, max: 100, step: 1, unit: '个', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'dynamic_eq_threshold', group: '视觉频段', label: '动态均衡阈值', status: 'implemented', min: 0, max: 20, step: 0.1, unit: '刻度', description: '限制视频空间频段调制的激活阈值与幅度。' }),
  numberDefinition({ section: 'advanced', field: 'channel_offset_percent', group: '视觉频段', label: '通道偏移', status: 'implemented', min: -10, max: 10, step: 0.1, unit: '%', description: implementedAdvancedDescription }),
  { section: 'advanced', field: 'space_dimension', group: '视觉频段', label: '空间维度', kind: 'select', status: 'implemented', description: implementedAdvancedDescription, options: [
    { key: '1', label: '1 维', value: 1 }, { key: '2', label: '2 维', value: 2 }, { key: '3', label: '3 维', value: 3 },
  ] },
  numberDefinition({ section: 'advanced', field: 'frequency_space_x_offset_px', group: '视觉频段', label: '频率空间 X 偏移', status: 'implemented', min: -10, max: 10, step: 0.1, unit: 'px', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'frequency_space_y_offset_px', group: '视觉频段', label: '频率空间 Y 偏移', status: 'implemented', min: -10, max: 10, step: 0.1, unit: 'px', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'frame_perturbation_probability_percent', group: '视觉频段', label: '帧扰动概率', status: 'implemented', min: 0, max: 20, step: 0.1, unit: '%', description: implementedAdvancedDescription }),

  booleanDefinition({ section: 'advanced', field: 'random_graphic_enabled', group: '挂件与画中画', label: '随机几何挂件', status: 'implemented', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'random_graphic_count', group: '挂件与画中画', label: '随机图形数量', status: 'implemented', min: 1, max: 32, step: 1, unit: '个', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'random_graphic_opacity_percent', group: '挂件与画中画', label: '随机图形透明度', status: 'implemented', min: 0, max: 50, step: 0.1, unit: '%', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'random_graphic_size_px', group: '挂件与画中画', label: '随机图形大小', status: 'implemented', min: 1, max: 64, step: 0.1, unit: 'px', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'abstract_face_count', group: '挂件与画中画', label: '抽象人脸数量', status: 'implemented', min: 0, max: 10, step: 1, unit: '个', description: '已接入抽象几何脸标记，不读取或合成真人素材。' }),
  numberDefinition({ section: 'advanced', field: 'abstract_face_size_percent', group: '挂件与画中画', label: '抽象人脸大小', status: 'implemented', min: 1, max: 10, step: 0.1, unit: '%/宽', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'abstract_face_opacity_percent', group: '挂件与画中画', label: '抽象人脸透明度', status: 'implemented', min: 0, max: 30, step: 0.1, unit: '%', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'overlay_offset_px', group: '挂件与画中画', label: '挂件画面偏移', status: 'implemented', min: -10, max: 10, step: 0.1, unit: 'px', description: implementedAdvancedDescription }),
  booleanDefinition({ section: 'advanced', field: 'picture_in_picture_enabled', group: '挂件与画中画', label: '画中画切片', status: 'implemented', description: '复用当前源视频构建同时间基画中画。' }),
  numberDefinition({ section: 'advanced', field: 'picture_in_picture_scale_percent', group: '挂件与画中画', label: '画中画比例', status: 'implemented', min: 10, max: 50, step: 0.1, unit: '%', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'picture_in_picture_opacity_percent', group: '挂件与画中画', label: '画中画透明度', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'picture_in_picture_rotation_degrees', group: '挂件与画中画', label: '画中画旋转', status: 'implemented', min: -15, max: 15, step: 0.1, unit: '°', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'picture_in_picture_pixel_jitter_px', group: '挂件与画中画', label: '画中画像素扰动', status: 'implemented', min: 0, max: 4, step: 0.01, unit: 'px', description: implementedAdvancedDescription }),
  booleanDefinition({ section: 'advanced', field: 'picture_in_picture_timeline_locked', group: '挂件与画中画', label: '画中画时间轴锁定', status: 'implemented', description: '锁定时跟随主时间轴；关闭时使用独立延迟时间轴。' }),

  numberDefinition({ section: 'advanced', field: 'slice_length_ms', group: '切片与局部效果', label: '切片长度', status: 'implemented', min: 500, max: 10_000, step: 100, unit: 'ms', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'slice_min_length_ms', group: '切片与局部效果', label: '源片段最小长度', status: 'implemented', min: 1_000, max: 60_000, step: 100, unit: 'ms', description: '已用于画中画独立时间轴的取段与回绕长度。' }),
  numberDefinition({ section: 'advanced', field: 'slice_trigger_interval_ms', group: '切片与局部效果', label: '切片触发间隔', status: 'implemented', min: 5_000, max: 120_000, step: 100, unit: 'ms', description: implementedAdvancedDescription }),
  booleanDefinition({ section: 'advanced', field: 'local_blur_enabled', group: '切片与局部效果', label: '局部模糊', status: 'implemented', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'local_blur_region_percent', group: '切片与局部效果', label: '局部模糊区域', status: 'implemented', min: 5, max: 50, step: 0.1, unit: '%', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'local_blur_radius_px', group: '切片与局部效果', label: '局部模糊半径', status: 'implemented', min: 0.1, max: 16, step: 0.1, unit: 'px', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'local_blur_interval_ms', group: '切片与局部效果', label: '局部模糊间隔', status: 'implemented', min: 500, max: 60_000, step: 100, unit: 'ms', description: implementedAdvancedDescription }),
  booleanDefinition({ section: 'advanced', field: 'edge_fill_enabled', group: '切片与局部效果', label: '边缘填充', status: 'implemented', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'edge_feather_percent', group: '切片与局部效果', label: '边缘羽化', status: 'implemented', min: 0, max: 100, step: 0.1, unit: '%', description: '已通过边缘掩码模糊后 maskedmerge 完成真实羽化。' }),
  booleanDefinition({ section: 'advanced', field: 'transform_smoothing_enabled', group: '切片与局部效果', label: '变换平滑', status: 'implemented', description: '已对动态裁剪变换应用平滑过渡。' }),
  numberDefinition({ section: 'advanced', field: 'transform_smoothing_duration_ms', group: '切片与局部效果', label: '变换平滑时长', status: 'implemented', min: 50, max: 5_000, step: 10, unit: 'ms', description: '控制动态裁剪 smoothstep 过渡时长。' }),
  booleanDefinition({ section: 'advanced', field: 'highlight_perturbation_enabled', group: '切片与局部效果', label: '高光扰动', status: 'implemented', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'highlight_perturbation_interval_ms', group: '切片与局部效果', label: '高光扰动间隔', status: 'implemented', min: 500, max: 60_000, step: 100, unit: 'ms', description: implementedAdvancedDescription }),
  booleanDefinition({ section: 'advanced', field: 'asynchronous_rotation_enabled', group: '切片与局部效果', label: '异步旋转', status: 'implemented', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'asynchronous_rotation_min_degrees', group: '切片与局部效果', label: '异步旋转最小角度', status: 'implemented', min: -15, max: 15, step: 0.1, unit: '°', description: implementedAdvancedDescription }),
  numberDefinition({ section: 'advanced', field: 'asynchronous_rotation_max_degrees', group: '切片与局部效果', label: '异步旋转最大角度', status: 'implemented', min: -15, max: 15, step: 0.1, unit: '°', description: implementedAdvancedDescription }),
];

export const MEDIA_PARAMETER_DEFINITIONS: Readonly<Record<MediaParameterSection, readonly MediaParameterDefinition[]>> = {
  video: VIDEO_PARAMETER_DEFINITIONS,
  audio: AUDIO_PARAMETER_DEFINITIONS,
  advanced: ADVANCED_PARAMETER_DEFINITIONS,
};

export function getMediaParameterPath(definition: MediaParameterDefinition): MediaParameterPath {
  return `${definition.section}.${definition.field}`;
}
