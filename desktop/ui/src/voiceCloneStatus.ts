export type VoiceClonePrepareStatusInput = {
  hasSource: boolean;
  workerAvailable: boolean;
  workerReason: string | null | undefined;
  status: string;
};

export type VoiceCloneAutoPrepareInput = {
  sourcePath: string | null | undefined;
  playbackGeneration: number | null | undefined;
  status: string;
  triggeredKey: string | null | undefined;
};

export type VoiceCloneReplaceDisabledInput = {
  hasSource: boolean;
  playbackState: string;
  positionMs: number | null | undefined;
  workerAvailable: boolean;
  workerReason: string | null | undefined;
  status: string;
  audioProcessingBlocked: boolean;
  realtimeAudioBusy: boolean;
  textError: string | null;
};

export type VoiceClonePreGenerationTriggerInput = {
  sourceReady: boolean;
  sourceGeneration: number | null | undefined;
  presetCount: number;
  presetTextRevision: number;
  batchStatus: string;
  replacementStatus: string;
  playbackStatus: string;
  lastStartedKey: string | null | undefined;
};

export type VoiceClonePreGenerationSummaryInput = {
  status: string;
  total: number;
  completed: number;
  failed: number;
};

export type VoiceClonePreGenerationResultInput = {
  cancelled: boolean;
  mounted: boolean;
  requestedGeneration: number | null | undefined;
  currentGeneration: number | null | undefined;
  resultGeneration: number | null | undefined;
};

export type VoiceClonePreGenerationRetryInput = {
  failedKey: string;
  currentKey: string | null | undefined;
  lastRetriedKey: string | null | undefined;
};

export function getVoiceClonePreGenerationTriggerKey(
  sourceGeneration: number | null | undefined,
  presetTextRevision: number,
): string | null {
  return sourceGeneration === null || sourceGeneration === undefined
    ? null
    : `${sourceGeneration}:r${presetTextRevision}`;
}

export function shouldStartVoiceClonePreGeneration(input: VoiceClonePreGenerationTriggerInput): boolean {
  const key = getVoiceClonePreGenerationTriggerKey(input.sourceGeneration, input.presetTextRevision);
  return input.sourceReady
    && input.presetCount > 0
    && input.presetCount <= 10
    && input.batchStatus !== 'generating'
    && !['preparing', 'generating'].includes(input.replacementStatus)
    && !['preparing', 'playing'].includes(input.playbackStatus)
    && key !== null
    && key !== input.lastStartedKey;
}

export function shouldAcceptVoiceClonePreGenerationResult(
  input: VoiceClonePreGenerationResultInput,
): boolean {
  return !input.cancelled
    && input.mounted
    && input.requestedGeneration !== null
    && input.requestedGeneration !== undefined
    && input.requestedGeneration === input.currentGeneration
    && input.requestedGeneration === input.resultGeneration;
}

export function shouldRetryVoiceClonePreGeneration(
  input: VoiceClonePreGenerationRetryInput,
): boolean {
  return input.failedKey === input.currentKey && input.failedKey !== input.lastRetriedKey;
}

export function getVoiceClonePreGenerationBusyReason(status: string): string | null {
  return status === 'generating' ? '正在批量准备文案人声' : null;
}

export function getVoiceClonePreGenerationSummary({
  status,
  total,
  completed,
  failed,
}: VoiceClonePreGenerationSummaryInput): string {
  if (failed > 0) return `${completed - failed} 条完成，${failed} 条失败`;
  if (status === 'ready') return `${completed} 条文案已准备`;
  if (status === 'generating') return `正在准备 ${completed}/${total}`;
  return total === 0 ? '等待文案生成' : `等待准备 ${completed}/${total}`;
}

export function getVoiceClonePreGenerationItemStatusLabel(status: string): string {
  switch (status) {
    case 'generating':
      return '生成中';
    case 'cached':
    case 'generated':
      return '已准备';
    case 'failed':
      return '生成失败';
    case 'cancelled':
      return '已取消';
    default:
      return '等待生成';
  }
}

export function shouldMuteOriginalAudioForVoiceClonePlayback(status: string): boolean {
  return status === 'playing';
}

export function shouldAutoReplayVoiceClonePlaybackOnLoop(): boolean {
  return false;
}

export function isVoiceCloneModelLoading(status: string, phase: string | null | undefined): boolean {
  return status === 'preparing' && phase === 'loading-local-model';
}

export function getVoiceCloneAutoPrepareKey(
  sourcePath: string | null | undefined,
  playbackGeneration: number | null | undefined,
): string | null {
  const normalizedPath = sourcePath?.trim();
  if (!normalizedPath || playbackGeneration === null || playbackGeneration === undefined) {
    return null;
  }
  return `${normalizedPath}:g${playbackGeneration}`;
}

export function shouldAutoPrepareVoiceCloneSource({
  sourcePath,
  playbackGeneration,
  status,
  triggeredKey,
}: VoiceCloneAutoPrepareInput): boolean {
  if (status !== 'idle') return false;
  const sourceKey = getVoiceCloneAutoPrepareKey(sourcePath, playbackGeneration);
  return sourceKey !== null && sourceKey !== triggeredKey;
}

export function getVoiceCloneReplaceDisabledReason({
  hasSource,
  playbackState,
  positionMs,
  workerAvailable,
  workerReason,
  status,
  audioProcessingBlocked,
  realtimeAudioBusy,
  textError,
}: VoiceCloneReplaceDisabledInput): string | null {
  if (!hasSource) return '请先导入一个 MP4';
  if (playbackState !== 'playing') return '请先让最终效果窗口保持播放';
  if (positionMs === null || positionMs === undefined) return '播放器位置尚未同步，请打开最终效果窗口';
  if (!workerAvailable) return workerReason ?? '固定话术 Worker 不可用';
  if (status === 'preparing' || status === 'generating') return '固定话术 Worker 正在执行';
  if (status !== 'ready' && status !== 'playing') return '请先准备人声';
  if (audioProcessingBlocked) return '请先应用当前普通声音处理参数';
  if (realtimeAudioBusy) return '当前实时音频正在占用';
  return textError;
}

export function getVoiceClonePrepareDisabledReason({
  hasSource,
  workerAvailable,
  workerReason,
  status,
}: VoiceClonePrepareStatusInput): string | null {
  if (!hasSource) return '请先导入一个 MP4';
  if (!workerAvailable) return workerReason ?? '固定话术 Worker 不可用';
  if (status === 'preparing' || status === 'generating') return '固定话术 Worker 正在执行';
  return null;
}

export function getVoiceCloneIdleNotice(): string {
  return '人声模型和运行环境由安装包提供，首次使用会加载模型，请稍候。';
}
