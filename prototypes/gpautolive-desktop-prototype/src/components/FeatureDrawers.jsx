import { AdvancedAudioDrawer } from "./AdvancedAudioDrawer.jsx";
import { FixedSpeechDrawer } from "./FixedSpeechDrawer.jsx";
import { InterruptionDrawer } from "./InterruptionDrawer.jsx";

export function FeatureDrawers({
  activeDrawer,
  advancedAudio,
  interruption,
  fixedSpeech,
  audioEnabled,
  onClose,
  onAudioEnabledChange,
  onAdvancedAudioChange,
  onAdvancedAudioRegenerate,
  onAdvancedAudioCleanup,
  onInterruptionChange,
  onFixedSpeechChange,
  onFixedSpeechPreview,
  onSave,
}) {
  return (
    <>
      <AdvancedAudioDrawer
        open={activeDrawer === "advancedAudio"}
        settings={{
          ...advancedAudio,
          actualOutputLabel: audioEnabled ? "PortAudio" : "WebView",
          actualMixLabel: advancedAudio.mixEnabled ? `${advancedAudio.mixPickMin}–${advancedAudio.mixPickMax} 条支路` : "1 条支路",
        }}
        processingEnabled={audioEnabled}
        onProcessingEnabledChange={onAudioEnabledChange}
        onChange={onAdvancedAudioChange}
        onRegenerate={onAdvancedAudioRegenerate}
        onCleanup={onAdvancedAudioCleanup}
        onClose={onClose}
        onSave={() => onSave?.("advancedAudio")}
      />

      <InterruptionDrawer
        open={activeDrawer === "interruption"}
        settings={{ ...interruption, actualAudioOutputLabel: audioEnabled ? "PortAudio" : "WebView" }}
        onChange={onInterruptionChange}
        onClose={onClose}
        onSave={() => onSave?.("interruption")}
      />

      <FixedSpeechDrawer
        open={activeDrawer === "fixedSpeech"}
        settings={fixedSpeech}
        onChange={onFixedSpeechChange}
        onClose={onClose}
        onPreview={onFixedSpeechPreview}
        onSave={() => onSave?.("fixedSpeech")}
      />
    </>
  );
}
