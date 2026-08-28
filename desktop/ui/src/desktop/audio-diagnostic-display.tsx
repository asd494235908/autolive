import { Typography } from 'antd';
import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from 'react';
import {
  DIAGNOSTIC_STALE_AFTER_MS,
  DIAGNOSTIC_UI_COMMIT_INTERVAL_MS,
  getDiagnosticStatus,
  isDiagnosticMessage,
  selectDiagnosticMessage,
  type DiagnosticMessage,
} from './audio-diagnostic-policy';

export type AudioDiagnosticSummary = {
  fresh: boolean;
  message: DiagnosticMessage | null;
};

export type AudioDiagnosticDisplayHandle = {
  acceptDiagnostic: (value: unknown) => boolean;
};

type AudioDiagnosticDisplayProps = {
  active: boolean;
  interludePlaybackActive: boolean;
  onSummaryChange: (summary: AudioDiagnosticSummary) => void;
};

function drawDiagnosticCanvas(
  canvas: HTMLCanvasElement | null,
  samples: number[],
  stroke: string,
  fill: string,
) {
  if (!canvas) return;
  const context = canvas.getContext('2d');
  if (!context) return;
  context.clearRect(0, 0, canvas.width, canvas.height);
  context.strokeStyle = stroke;
  context.fillStyle = fill;
  context.lineWidth = 2;
  context.beginPath();
  samples.forEach((sample, index) => {
    const x = samples.length <= 1 ? 0 : (index / (samples.length - 1)) * canvas.width;
    const y = canvas.height / 2 - sample * canvas.height * 0.45;
    if (index === 0) context.moveTo(x, y);
    else context.lineTo(x, y);
  });
  context.stroke();
  context.fillRect(0, canvas.height - 1, canvas.width, 1);
}

export const AudioDiagnosticDisplay = memo(forwardRef<
  AudioDiagnosticDisplayHandle,
  AudioDiagnosticDisplayProps
>(function AudioDiagnosticDisplay({ active, interludePlaybackActive, onSummaryChange }, ref) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const currentRef = useRef<DiagnosticMessage | null>(null);
  const activeRef = useRef(active);
  const interludePlaybackActiveRef = useRef(interludePlaybackActive);
  const onSummaryChangeRef = useRef(onSummaryChange);
  const lastCommitAtRef = useRef(0);
  const commitTimerRef = useRef<number | null>(null);
  const staleTimerRef = useRef<number | null>(null);
  const [summary, setSummary] = useState<AudioDiagnosticSummary>({ fresh: false, message: null });

  activeRef.current = active;
  interludePlaybackActiveRef.current = interludePlaybackActive;
  onSummaryChangeRef.current = onSummaryChange;

  const clearCommitTimer = useCallback(() => {
    if (commitTimerRef.current === null) return;
    window.clearTimeout(commitTimerRef.current);
    commitTimerRef.current = null;
  }, []);

  const clearStaleTimer = useCallback(() => {
    if (staleTimerRef.current === null) return;
    window.clearTimeout(staleTimerRef.current);
    staleTimerRef.current = null;
  }, []);

  const commitSummary = useCallback((message: DiagnosticMessage | null, fresh: boolean) => {
    const next = { message, fresh };
    lastCommitAtRef.current = Date.now();
    setSummary(next);
    onSummaryChangeRef.current(next);
  }, []);

  const drawCurrent = useCallback((message: DiagnosticMessage | null) => {
    const usePortAudioColor = message?.source === 'portaudio-mixed-pcm';
    drawDiagnosticCanvas(
      canvasRef.current,
      interludePlaybackActiveRef.current ? [0, 0] : message?.line ?? [0, 0],
      usePortAudioColor ? '#1677ff' : '#fa8c16',
      usePortAudioColor ? 'rgba(22, 119, 255, 0.12)' : 'rgba(250, 140, 22, 0.12)',
    );
  }, []);

  const scheduleSummary = useCallback((message: DiagnosticMessage, fresh: boolean, nowMs: number) => {
    const elapsedMs = nowMs - lastCommitAtRef.current;
    if (summary.message === null || summary.fresh !== fresh || elapsedMs >= DIAGNOSTIC_UI_COMMIT_INTERVAL_MS) {
      clearCommitTimer();
      commitSummary(message, fresh);
      return;
    }
    if (commitTimerRef.current !== null) return;
    commitTimerRef.current = window.setTimeout(() => {
      commitTimerRef.current = null;
      const latest = currentRef.current;
      if (!activeRef.current || latest === null) return;
      const latestFresh = Date.now() - latest.sent_at_ms < DIAGNOSTIC_STALE_AFTER_MS;
      commitSummary(latest, latestFresh);
    }, DIAGNOSTIC_UI_COMMIT_INTERVAL_MS - elapsedMs);
  }, [clearCommitTimer, commitSummary, summary.fresh, summary.message]);

  const acceptDiagnostic = useCallback((value: unknown): boolean => {
    if (!activeRef.current || !isDiagnosticMessage(value)) return false;
    const nowMs = Date.now();
    const selected = selectDiagnosticMessage(currentRef.current, value, nowMs);
    currentRef.current = selected;
    drawCurrent(selected);

    clearStaleTimer();
    const fresh = nowMs - selected.sent_at_ms < DIAGNOSTIC_STALE_AFTER_MS;
    if (fresh) {
      staleTimerRef.current = window.setTimeout(() => {
        staleTimerRef.current = null;
        if (currentRef.current !== selected) return;
        clearCommitTimer();
        commitSummary(selected, false);
      }, Math.max(0, selected.sent_at_ms + DIAGNOSTIC_STALE_AFTER_MS - nowMs));
    }
    scheduleSummary(selected, fresh, nowMs);
    return true;
  }, [clearCommitTimer, clearStaleTimer, commitSummary, drawCurrent, scheduleSummary]);

  useImperativeHandle(ref, () => ({ acceptDiagnostic }), [acceptDiagnostic]);

  useEffect(() => {
    drawCurrent(currentRef.current);
  }, [drawCurrent, interludePlaybackActive]);

  useEffect(() => {
    if (active) return;
    clearCommitTimer();
    clearStaleTimer();
    drawDiagnosticCanvas(canvasRef.current, [0, 0], '#fa8c16', 'rgba(250, 140, 22, 0.12)');
    commitSummary(currentRef.current, false);
  }, [active, clearCommitTimer, clearStaleTimer, commitSummary]);

  useEffect(() => () => {
    clearCommitTimer();
    clearStaleTimer();
  }, [clearCommitTimer, clearStaleTimer]);

  const status = interludePlaybackActive
    ? '插话播放中 · 普通声音波形暂停显示'
    : getDiagnosticStatus(summary.message, summary.fresh);

  return (
    <>
      <canvas
        ref={canvasRef}
        width={320}
        height={64}
        aria-label={interludePlaybackActive ? '插话播放中，普通声音波形为直线' : '声音波形'}
        className="desktop-diagnostic-canvas"
      />
      <Typography.Text className="desktop-muted">{status}</Typography.Text>
    </>
  );
}));
