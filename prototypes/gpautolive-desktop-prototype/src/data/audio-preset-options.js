const AUDIO_PRESET_LABELS = [
  '自然平直', '微抬增益', '微降增益', '暖低频', '亮高频', '中频突出',
  '轻上移音高', '轻下移音高', '轻颤音', '相位微扰', '轻混响', '干声收紧',
  '轻降噪', '底噪纹理', '淡入淡出', '自然微变', '音色着色', '空间感',
  '调制组合', '综合微扰', '明显加轨与空间（手动）', '明显变调与音色（手动）',
];

export const AUDIO_PRESET_OPTIONS = AUDIO_PRESET_LABELS.map((label, index) => {
  const id = `p${String(index + 1).padStart(2, '0')}`;
  return { value: id, label: `${index + 1}. ${label}` };
});

export const DEFAULT_AUDIO_PRESET_IDS = AUDIO_PRESET_OPTIONS.slice(0, 20).map(({ value }) => value);
