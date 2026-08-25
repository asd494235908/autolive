import type { ReactNode } from 'react';

export type NaturalVoiceMode = 'original' | 'natural_dynamic';

export interface AudioEffectParams extends Record<string, unknown> {
  natural_voice_mode: NaturalVoiceMode;
  random_change_period_ms: number;
  pitch_shift_semitones: number;
  spectral_perturbation_percent: number;
  environment_noise_percent: number;
  environment_noise_dbfs: number;
  mfcc_shift_percent: number;
  phase_perturbation_percent: number;
  loudness_adjustment_db: number;
  input_gain_db: number;
  output_gain_db: number;
  playback_speed: number;
  low_eq_db: number;
  mid_eq_db: number;
  high_eq_db: number;
  noise_reduction_percent: number;
  ambient_sound_mix_percent: number;
  fade_in_ms: number;
  fade_out_ms: number;
  dry_wet_percent: number;
  reverb_wet_percent: number;
  mfcc_dimensions: number;
  snr_variation_db: number;
  formant_shift_percent: number;
  vibrato_frequency_hz: number;
  vibrato_depth_percent: number;
  spectrum_blind_spot_percent: number;
  snr_target_db: number | null;
  current_formant_hz: number | null;
  filter_q: number;
  sample_rate_hz: number | null;
  output_bitrate_kbps: number;
  voice_library_id: string | null;
  high_frequency_perturbation_enabled: boolean;
  high_frequency_perturbation_interval_ms: number;
  high_frequency_perturbation_strength_percent: number;
  high_frequency_perturbation_level_db: number;
}

export interface VideoEffectParams extends Record<string, unknown> {
  brightness_percent: number;
  saturation_percent: number;
  blur_radius_px: number;
  contrast_percent: number;
  hue_rotation_degrees: number;
  sharpen_percent: number;
  noise_percent: number;
  detail_enhancement_percent: number;
  crop_edge_smoothing: number;
  frame_rate_jitter_percent: number;
  frame_rate_perturbation_frequency_hz: number;
  frame_rate_perturbation_amplitude_fps: number;
  pixel_scale_percent: number;
  pixel_jitter_px: number;
  dynamic_crop_percent: number;
  frame_inner_perturbation_percent: number;
  frame_inter_perturbation_percent: number;
  space_x_offset_px: number;
  space_y_offset_px: number;
  color_space_conversion_strength_percent: number;
  color_space_conversion_enabled: boolean;
  horizontal_flip_enabled: boolean;
  vertical_flip_enabled: boolean;
  rotation_degrees: number;
  vignette_percent: number;
  highlights_percent: number;
  shadows_percent: number;
  red_channel_lock_enabled: boolean;
  edge_softness_percent: number;
  image_repair_enabled: boolean;
  image_repair_strength_percent: number;
  frame_rate_lock_enabled: boolean;
}

export interface AdvancedEffectParams extends Record<string, unknown> {
  band_weights: Record<string, number>;
  target_frequency_hz: number | null;
  core_frequency_hz: number | null;
  wave_intensity: number;
  wave_level: number;
  wave_grain_count: number;
  dynamic_eq_threshold: number;
  channel_offset_percent: number;
  space_dimension: number;
  frequency_space_x_offset_px: number;
  frequency_space_y_offset_px: number;
  frame_perturbation_probability_percent: number;
  random_graphic_opacity_percent: number;
  random_graphic_size_px: number;
  abstract_face_count: number;
  abstract_face_size_percent: number;
  abstract_face_opacity_percent: number;
  overlay_offset_px: number;
  slice_length_ms: number;
  slice_min_length_ms: number;
  slice_trigger_interval_ms: number;
  random_graphic_enabled: boolean;
  random_graphic_count: number;
  picture_in_picture_enabled: boolean;
  picture_in_picture_scale_percent: number;
  picture_in_picture_opacity_percent: number;
  picture_in_picture_rotation_degrees: number;
  picture_in_picture_pixel_jitter_px: number;
  picture_in_picture_timeline_locked: boolean;
  local_blur_enabled: boolean;
  local_blur_region_percent: number;
  local_blur_radius_px: number;
  local_blur_interval_ms: number;
  edge_fill_enabled: boolean;
  edge_feather_percent: number;
  transform_smoothing_enabled: boolean;
  transform_smoothing_duration_ms: number;
  highlight_perturbation_enabled: boolean;
  highlight_perturbation_interval_ms: number;
  asynchronous_rotation_enabled: boolean;
  asynchronous_rotation_min_degrees: number;
  asynchronous_rotation_max_degrees: number;
}

export interface MediaEffectParams {
  audio: AudioEffectParams;
  video: VideoEffectParams;
  advanced: AdvancedEffectParams;
}

export type MediaParameterSection = keyof MediaEffectParams;
export type MediaParameterStatus = 'implemented' | 'planned' | 'pending_confirmation';
export type MediaParameterPath = `${MediaParameterSection}.${string}`;
export type EditableAudioParameterField =
  | 'natural_voice_mode'
  | 'voice_library_id'
  | 'snr_target_db';

export interface AudioParameterControlsProps {
  value: AudioEffectParams;
  disabled: boolean;
  ambientSoundPath: string | null;
  onChange: <Field extends EditableAudioParameterField>(
    field: Field,
    value: AudioEffectParams[Field],
  ) => void;
  onChooseAmbientSound: () => void | Promise<void>;
  onClearAmbientSound: () => void;
}

export interface MediaParameterPanelsProps {
  value: MediaEffectParams;
  loading?: boolean;
  error?: string | null;
  statusOverrides?: Partial<Record<MediaParameterPath, MediaParameterStatus>>;
  audioControls?: ReactNode;
}
