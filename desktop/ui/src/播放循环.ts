type 播放重启判断输入 = {
  restartToken?: string | number | null;
  lastRestartToken?: string | number | null;
  mediaGeneration?: number | null;
  lastRestartGeneration?: number | null;
  ended: boolean;
  currentTime?: number;
  duration?: number;
};

const 接近结束阈值秒 = 0.05;

function 读取当前重启令牌({
  restartToken,
  mediaGeneration,
}: Pick<播放重启判断输入, 'restartToken' | 'mediaGeneration'>): string | number | null {
  if (restartToken !== undefined) {
    return restartToken;
  }
  return mediaGeneration ?? null;
}

function 读取上次重启令牌({
  lastRestartToken,
  lastRestartGeneration,
}: Pick<播放重启判断输入, 'lastRestartToken' | 'lastRestartGeneration'>): string | number | null {
  if (lastRestartToken !== undefined) {
    return lastRestartToken;
  }
  return lastRestartGeneration ?? null;
}

function 令牌有效(令牌: string | number | null): 令牌 is string | number {
  return typeof 令牌 === 'string' ? 令牌.length > 0 : Number.isFinite(令牌);
}

export function shouldRestartPlayback({
  restartToken,
  lastRestartToken,
  mediaGeneration,
  lastRestartGeneration,
  ended,
  currentTime,
  duration,
}: 播放重启判断输入): boolean {
  const currentRestartToken = 读取当前重启令牌({ restartToken, mediaGeneration });
  const previousRestartToken = 读取上次重启令牌({
    lastRestartToken,
    lastRestartGeneration,
  });

  if (!令牌有效(currentRestartToken)) {
    return false;
  }

  if (
    previousRestartToken !== null &&
    previousRestartToken !== undefined &&
    (!令牌有效(previousRestartToken) || currentRestartToken === previousRestartToken)
  ) {
    return false;
  }

  if (ended) {
    return true;
  }

  return (
    Number.isFinite(currentTime) &&
    Number.isFinite(duration) &&
    (duration as number) > 0 &&
    (currentTime as number) >= (duration as number) - 接近结束阈值秒
  );
}
