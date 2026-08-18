export const FIXED_SPEECH_MAX_TEXT_LENGTH = 500;

const FIXED_SPEECH_STATUSES = new Set([
  'starting',
  'playing',
  'completed',
  'cancelled',
  'failed',
]);

export type FixedSpeechStatus = 'starting' | 'playing' | 'completed' | 'cancelled' | 'failed';

export type FixedSpeechCommandMessage =
  | {
      version: 1;
      type: 'fixed-speech-command';
      action: 'speak';
      operation_id: string;
      text: string;
    }
  | {
      version: 1;
      type: 'fixed-speech-command';
      action: 'cancel';
      operation_id: string;
    };

export type FixedSpeechStatusMessage = {
  version: 1;
  type: 'fixed-speech-status';
  operation_id: string;
  status: FixedSpeechStatus;
  error: string | null;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object';
}

function isOperationId(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0 && value.length <= 128;
}

export function countUnicodeCharacters(value: string): number {
  return Array.from(value).length;
}

export function getFixedSpeechTextError(text: string): string | null {
  const trimmed = text.trim();
  if (!trimmed) return '文本不能为空';
  if (countUnicodeCharacters(trimmed) > FIXED_SPEECH_MAX_TEXT_LENGTH) {
    return `文本最多 ${FIXED_SPEECH_MAX_TEXT_LENGTH} 个字符`;
  }
  return null;
}

export function isFixedSpeechCommandMessage(value: unknown): value is FixedSpeechCommandMessage {
  if (!isRecord(value) || value.version !== 1 || value.type !== 'fixed-speech-command') return false;
  if (!isOperationId(value.operation_id)) return false;
  if (value.action === 'cancel') return true;
  return value.action === 'speak'
    && typeof value.text === 'string'
    && getFixedSpeechTextError(value.text) === null;
}

export function isFixedSpeechStatusMessage(value: unknown): value is FixedSpeechStatusMessage {
  if (!isRecord(value) || value.version !== 1 || value.type !== 'fixed-speech-status') return false;
  return isOperationId(value.operation_id)
    && typeof value.status === 'string'
    && FIXED_SPEECH_STATUSES.has(value.status)
    && (value.error === null || (typeof value.error === 'string' && value.error.length <= 500));
}

export function selectLocalSpeechVoice<T extends Pick<SpeechSynthesisVoice, 'default' | 'lang' | 'localService'>>(
  voices: readonly T[],
  preferredLanguage = 'zh-CN',
): T | null {
  const localVoices = voices.filter(({ localService }) => localService);
  if (localVoices.length === 0) return null;
  const preferred = preferredLanguage.toLowerCase();
  const baseLanguage = preferred.split('-')[0];
  return localVoices.find(({ lang }) => lang.toLowerCase() === preferred)
    ?? localVoices.find(({ lang }) => lang.toLowerCase().split('-')[0] === baseLanguage)
    ?? localVoices.find(({ default: isDefault }) => isDefault)
    ?? localVoices[0];
}
