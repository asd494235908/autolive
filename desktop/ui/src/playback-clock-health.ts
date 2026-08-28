export const PLAYBACK_CLOCK_STALL_MS = 1_500;
export const PLAYBACK_CLOCK_RECOVERY_TIMEOUT_MS = 4_000;
export const PLAYBACK_CLOCK_ADVANCE_EPSILON_SEC = 0.02;

export type PlaybackClockStatus = 'healthy' | 'buffering' | 'recovering' | 'stalled';

export type PlaybackClockHealth = {
  identity: string;
  status: PlaybackClockStatus;
  lastPositionSec: number;
  lastAdvanceAtMs: number;
  recoveryAttempted: boolean;
  recoveryStartedAtMs: number | null;
};

export type PlaybackClockObservation = {
  identity: string;
  nowMs: number;
  currentTime: number;
  expectedPlaying: boolean;
  paused: boolean;
  seeking: boolean;
  ended: boolean;
  loadingGrace: boolean;
};

function safeTime(value: number): number {
  return Number.isFinite(value) && value >= 0 ? value : 0;
}

export function createPlaybackClockHealth(
  identity: string,
  currentTime: number,
  nowMs: number,
): PlaybackClockHealth {
  return {
    identity,
    status: 'healthy',
    lastPositionSec: safeTime(currentTime),
    lastAdvanceAtMs: nowMs,
    recoveryAttempted: false,
    recoveryStartedAtMs: null,
  };
}

export function resetPlaybackClockObservation(
  previous: PlaybackClockHealth,
  identity: string,
  currentTime: number,
  nowMs: number,
): PlaybackClockHealth {
  if (previous.identity !== identity) {
    return createPlaybackClockHealth(identity, currentTime, nowMs);
  }
  return {
    ...previous,
    status: previous.status === 'stalled' ? 'stalled' : 'healthy',
    lastPositionSec: safeTime(currentTime),
    lastAdvanceAtMs: nowMs,
    recoveryStartedAtMs: null,
  };
}

export function observePlaybackClock(
  previous: PlaybackClockHealth,
  observation: PlaybackClockObservation,
): { state: PlaybackClockHealth; recoveryRequested: boolean } {
  const currentTime = safeTime(observation.currentTime);
  if (previous.identity !== observation.identity) {
    return {
      state: createPlaybackClockHealth(observation.identity, currentTime, observation.nowMs),
      recoveryRequested: false,
    };
  }

  const advanced = Math.abs(currentTime - previous.lastPositionSec)
    >= PLAYBACK_CLOCK_ADVANCE_EPSILON_SEC;
  if (advanced) {
    return {
      state: {
        ...previous,
        status: 'healthy',
        lastPositionSec: currentTime,
        lastAdvanceAtMs: observation.nowMs,
        recoveryAttempted: false,
        recoveryStartedAtMs: null,
      },
      recoveryRequested: false,
    };
  }

  if (
    !observation.expectedPlaying
    || observation.ended
  ) {
    return {
      state: previous.status === 'recovering' || previous.status === 'stalled'
        ? previous
        : {
            ...previous,
            status: 'healthy',
            lastPositionSec: currentTime,
            lastAdvanceAtMs: observation.nowMs,
          },
      recoveryRequested: false,
    };
  }

  if (observation.loadingGrace) {
    return {
      state: previous.status === 'recovering'
        ? previous
        : {
            ...previous,
            status: 'buffering',
            lastPositionSec: currentTime,
          },
      recoveryRequested: false,
    };
  }

  if (observation.seeking) {
    return {
      state: previous.status === 'recovering'
        ? previous
        : {
            ...previous,
            status: 'healthy',
            lastPositionSec: currentTime,
            lastAdvanceAtMs: observation.nowMs,
          },
      recoveryRequested: false,
    };
  }

  if (previous.status === 'recovering') {
    const recoveryStartedAtMs = previous.recoveryStartedAtMs ?? observation.nowMs;
    return {
      state: observation.nowMs - recoveryStartedAtMs >= PLAYBACK_CLOCK_RECOVERY_TIMEOUT_MS
        ? { ...previous, status: 'stalled', recoveryStartedAtMs }
        : { ...previous, recoveryStartedAtMs },
      recoveryRequested: false,
    };
  }

  if (previous.status === 'stalled' || previous.recoveryAttempted) {
    return {
      state: { ...previous, status: 'stalled' },
      recoveryRequested: false,
    };
  }

  if (observation.nowMs - previous.lastAdvanceAtMs < PLAYBACK_CLOCK_STALL_MS) {
    return {
      state: previous.status === 'buffering' ? { ...previous, status: 'healthy' } : previous,
      recoveryRequested: false,
    };
  }

  return {
    state: {
      ...previous,
      status: 'recovering',
      recoveryAttempted: true,
      recoveryStartedAtMs: observation.nowMs,
    },
    recoveryRequested: true,
  };
}

export function failPlaybackClockRecovery(
  state: PlaybackClockHealth,
  identity: string,
): PlaybackClockHealth {
  return state.identity === identity && state.status === 'recovering'
    ? { ...state, status: 'stalled' }
    : state;
}
