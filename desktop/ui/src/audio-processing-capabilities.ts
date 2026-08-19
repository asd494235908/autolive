export type AudioCapabilityStatus = 'disabled' | 'configured' | 'runtime' | 'ready' | 'unsupported';

export type AudioCapabilityValues = {
  input_gain_db?: number | null;
  output_gain_db?: number | null;
  loudness_adjustment_db?: number | null;
  low_eq_db?: number | null;
  mid_eq_db?: number | null;
  high_eq_db?: number | null;
  pitch_shift_semitones?: number | null;
  playback_speed?: number | null;
  fade_in_ms?: number | null;
  fade_out_ms?: number | null;
  reverb_wet_percent?: number | null;
  noise_reduction_percent?: number | null;
  phase_perturbation_percent?: number | null;
  vibrato_frequency_hz?: number | null;
  vibrato_depth_percent?: number | null;
  environment_noise_percent?: number | null;
  environment_noise_dbfs?: number | null;
  filter_q?: number | null;
  dry_wet_percent?: number | null;
  ambient_sound_mix_percent?: number | null;
  spectral_perturbation_percent?: number | null;
  sample_rate_hz?: number | null;
  output_bitrate_kbps?: number | null;
};

export type AudioCapabilityRow = {
  key: string;
  label: string;
  value: number | null;
  unit: string;
  status: AudioCapabilityStatus;
  description: string;
};

const CONFIGURED_DESCRIPTION = '已写入 FFmpeg 参数，等待应用处理缓存';
const RUNTIME_DESCRIPTION = 'FFmpeg 处理缓存已生成并在播放链生效';

function finiteOrNull(value: number | null | undefined): number | null {
  return value !== null && value !== undefined && Number.isFinite(value) ? value : null;
}

export function buildAudioCapabilityRows(
  audio: AudioCapabilityValues | null,
  enabled: boolean,
  runtimeActive: boolean,
): AudioCapabilityRow[] {
  const runtimeStatus: AudioCapabilityStatus = !enabled ? 'disabled' : runtimeActive ? 'ready' : 'configured';
  const configuredStatus: AudioCapabilityStatus = enabled ? 'configured' : 'disabled';
  const values = audio ?? {};
  const gainParts = [values.input_gain_db, values.output_gain_db, values.loudness_adjustment_db]
    .filter((value): value is number => value !== null && value !== undefined && Number.isFinite(value));
  const totalGain = gainParts.length > 0 ? gainParts.reduce((sum, value) => sum + value, 0) : null;

  return [
    { key: 'total_gain_db', label: '总增益', value: totalGain, unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'input_gain_db', label: '输入增益', value: finiteOrNull(values.input_gain_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'output_gain_db', label: '输出增益', value: finiteOrNull(values.output_gain_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'loudness_adjustment_db', label: '响度调整', value: finiteOrNull(values.loudness_adjustment_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'low_eq_db', label: '低频 EQ', value: finiteOrNull(values.low_eq_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'mid_eq_db', label: '中频 EQ', value: finiteOrNull(values.mid_eq_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'high_eq_db', label: '高频 EQ', value: finiteOrNull(values.high_eq_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'pitch_shift_semitones', label: '音高', value: finiteOrNull(values.pitch_shift_semitones), unit: '半音', status: configuredStatus, description: CONFIGURED_DESCRIPTION },
    { key: 'playback_speed', label: '变速', value: finiteOrNull(values.playback_speed), unit: 'x', status: configuredStatus, description: CONFIGURED_DESCRIPTION },
    { key: 'fade_in_ms', label: '淡入', value: finiteOrNull(values.fade_in_ms), unit: 'ms', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'fade_out_ms', label: '淡出', value: finiteOrNull(values.fade_out_ms), unit: 'ms', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'reverb_wet_percent', label: '轻混响', value: finiteOrNull(values.reverb_wet_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'noise_reduction_percent', label: '降噪', value: finiteOrNull(values.noise_reduction_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'phase_perturbation_percent', label: '相位扰动', value: finiteOrNull(values.phase_perturbation_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'vibrato_frequency_hz', label: '颤音频率', value: finiteOrNull(values.vibrato_frequency_hz), unit: 'Hz', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'vibrato_depth_percent', label: '颤音深度', value: finiteOrNull(values.vibrato_depth_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'environment_noise_percent', label: '环境噪声', value: finiteOrNull(values.environment_noise_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'environment_noise_dbfs', label: '环境噪声电平', value: finiteOrNull(values.environment_noise_dbfs), unit: 'dBFS', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'filter_q', label: '滤波 Q', value: finiteOrNull(values.filter_q), unit: '', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'dry_wet_percent', label: '干湿比', value: finiteOrNull(values.dry_wet_percent), unit: '%', status: 'unsupported', description: '暂未支持；非默认值会被 Rust 媒体边界拒绝' },
    { key: 'ambient_sound_mix_percent', label: '环境声混合', value: finiteOrNull(values.ambient_sound_mix_percent), unit: '%', status: 'unsupported', description: '暂未支持；非默认值会被 Rust 媒体边界拒绝' },
    { key: 'spectral_perturbation_percent', label: '频谱微扰', value: finiteOrNull(values.spectral_perturbation_percent), unit: '%', status: 'unsupported', description: '暂未支持；非默认值会被 Rust 媒体边界拒绝' },
    { key: 'sample_rate_hz', label: '采样率', value: finiteOrNull(values.sample_rate_hz), unit: 'Hz', status: configuredStatus, description: CONFIGURED_DESCRIPTION },
    { key: 'output_bitrate_kbps', label: '输出码率', value: finiteOrNull(values.output_bitrate_kbps), unit: 'kbps', status: configuredStatus, description: CONFIGURED_DESCRIPTION },
  ];
}
