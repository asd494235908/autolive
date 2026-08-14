export type VoiceClonePrepareStatusInput = {
  hasSource: boolean;
  sourceHashStatus: string | null | undefined;
  workerAvailable: boolean;
  workerReason: string | null | undefined;
  status: string;
};

export function getVoiceClonePrepareDisabledReason({
  hasSource,
  sourceHashStatus,
  workerAvailable,
  workerReason,
  status,
}: VoiceClonePrepareStatusInput): string | null {
  if (!hasSource) return '请先导入一个 MP4';
  if (sourceHashStatus !== 'ready') {
    return sourceHashStatus === 'failed'
      ? '当前 MP4 的 SHA-256 计算失败，请重新导入视频'
      : '正在计算当前 MP4 的完整 SHA-256，请稍候';
  }
  if (!workerAvailable) return workerReason ?? '固定话术 Worker 不可用';
  if (status === 'preparing' || status === 'generating') return '固定话术 Worker 正在执行';
  return null;
}

export function getVoiceCloneIdleNotice(sourceHashStatus: string | null | undefined): string {
  if (sourceHashStatus !== 'ready') {
    return sourceHashStatus === 'failed'
      ? '当前 MP4 的 SHA-256 计算失败，请重新导入视频后再准备人声。'
      : '正在计算当前 MP4 的完整 SHA-256，完成后即可准备人声。';
  }
  return '人声模型和运行环境由安装包提供，首次使用会加载模型，请稍候。';
}
