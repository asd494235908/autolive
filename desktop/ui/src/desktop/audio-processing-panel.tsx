import type { ReactNode } from 'react';

export function AudioProcessingPanel({
  ariaLabel,
  actualOutput,
  processingStatus,
  currentPreset,
  children,
}: {
  ariaLabel: string;
  actualOutput: ReactNode;
  processingStatus: ReactNode;
  currentPreset: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="desktop-audio-summary" aria-label={ariaLabel}>
      <dl className="desktop-audio-facts">
        <div><dt>实际出口</dt><dd>{actualOutput}</dd></div>
        <div><dt>处理状态</dt><dd>{processingStatus}</dd></div>
        <div><dt>当前预设</dt><dd>{currentPreset}</dd></div>
      </dl>
      {children}
    </section>
  );
}
