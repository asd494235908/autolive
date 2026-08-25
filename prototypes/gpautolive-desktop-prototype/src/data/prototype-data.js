export const PARAMETER_PALETTE = [
  '#38B8F8',
  '#F89838',
  '#C888F8',
  '#E878F8',
  '#F8C818',
];

export const PARAMETER_STATUS = {
  implemented: { label: '已接入', tone: 'success' },
  planned: { label: '正式需求待实现', tone: 'warning' },
  pendingConfirmation: { label: '待确认', tone: 'default' },
};

export const STATUS_CARD_TEMPLATES = [
  { key: 'progress', label: '播放进度', icon: 'progress' },
  { key: 'pool', label: '循环次数', icon: 'pool' },
  { key: 'video', label: '视频处理', icon: 'video' },
  { key: 'audio', label: '普通声音', icon: 'audio' },
  { key: 'source', label: '源规格', icon: 'source' },
  { key: 'output', label: '最终效果窗口', icon: 'output' },
];

export const DEFAULT_PLAYBACK_POOL = {
  current: 1,
  total: 3,
  cycle: 12,
};

export const DEFAULT_CYCLE_STATES = {
  video: {
    range: '8–15 秒',
    changes: 12,
    progress: 64,
    hasNextPlan: true,
    stateLabel: '处理运行中',
  },
  audio: {
    range: '8–15 秒',
    changes: 9,
    progress: 42,
    hasNextPlan: true,
    stateLabel: 'PortAudio 运行中',
  },
};

export const VIDEO_PARAMETERS = [
  { key: 'brightness', label: '亮度', value: '+0.6%', progress: 51, status: 'implemented' },
  { key: 'contrast', label: '对比度', value: '99.8%', progress: 50, status: 'implemented' },
  { key: 'saturation', label: '饱和度', value: '100.2%', progress: 50, status: 'implemented' },
  { key: 'hue', label: '色相旋转', value: '+0.08°', progress: 50, status: 'implemented' },
  { key: 'sharpness', label: '锐化', value: '0.3%', progress: 1, status: 'implemented' },
  { key: 'pixelJitter', label: '像素级扰动', value: '0.04 px', progress: 2, status: 'implemented' },
  { key: 'frameRateLock', label: '源帧率 CFR 锁定', value: '已开启', progress: 100, status: 'implemented' },
  { key: 'imageRepair', label: '图像修复', value: '低感知', progress: 18, status: 'implemented' },
];

export const VISUAL_BAND_WEIGHTS = [
  [65, 1.01],
  [92, 0.99],
  [131, 1.0],
  [188, 1.02],
  [267, 0.98],
  [381, 1.01],
  [544, 1.0],
  [777, 0.99],
  [1110, 1.02],
  [1585, 1.0],
  [2263, 0.98],
  [20000, 1.01],
];

export const ADVANCED_PARAMETERS = [
  {
    key: 'visualBands',
    label: '固定视觉频段权重',
    value: '12 段',
    status: 'implemented',
    type: 'frequency',
    bands: VISUAL_BAND_WEIGHTS,
  },
  { key: 'targetFrequency', label: '目标频率', value: '544 Hz', progress: 45, status: 'implemented' },
  { key: 'waveIntensity', label: '波频强度', value: '0.12', progress: 12, status: 'implemented' },
  { key: 'pip', label: '同源画中画', value: '已开启', progress: 100, status: 'implemented' },
  { key: 'pipOpacity', label: '画中画透明度', value: '1.0%', progress: 1, status: 'implemented' },
  { key: 'sliceInterval', label: '切片触发间隔', value: '15 秒', progress: 13, status: 'implemented' },
];

export const AUDIO_PARAMETERS = [
  { key: 'naturalMode', label: '自然真人模式', value: '自然动态', progress: 100, status: 'implemented' },
  { key: 'pitch', label: '高质量音高', value: '+0.03 半音', progress: 51, status: 'implemented' },
  { key: 'formant', label: '共振峰偏移', value: '-0.12%', progress: 49, status: 'implemented' },
  { key: 'speed', label: '播放速度', value: '0.999 倍', progress: 50, status: 'implemented' },
  { key: 'noiseReduction', label: '降噪', value: '2.0%', progress: 2, status: 'implemented' },
  { key: 'ambientMix', label: '环境声素材混合', value: '1.5%', progress: 2, status: 'implemented' },
  { key: 'dryWet', label: '干湿比', value: '3.0%', progress: 3, status: 'implemented' },
  { key: 'mfcc', label: 'MFCC 偏移 / 维度', value: '+0.2% / 13 阶', progress: 33, status: 'implemented' },
  { key: 'snr', label: '目标信噪比', value: '自动', progress: 0, status: 'implemented' },
  { key: 'sampleRate', label: '目标采样率', value: '跟随源素材', progress: 100, status: 'implemented' },
];
