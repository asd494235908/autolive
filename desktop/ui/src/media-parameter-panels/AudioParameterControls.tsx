import { DeleteOutlined, FolderOpenOutlined } from '@ant-design/icons';
import { Button, Form, Input, InputNumber, Select, Tag, Typography } from 'antd';
import { useId } from 'react';

import type { AudioParameterControlsProps } from './media-parameter-types';

export function AudioParameterControls({
  value,
  disabled,
  ambientSoundPath,
  onChange,
  onChooseAmbientSound,
  onClearAmbientSound,
}: AudioParameterControlsProps) {
  const idBase = useId().replace(/:/g, '');
  const voiceLibraryHelpId = `${idBase}-voice-library-help`;
  const snrTargetHelpId = `${idBase}-snr-target-help`;
  const ambientSoundHelpId = `${idBase}-ambient-sound-help`;
  const voiceLibraryError = typeof value.voice_library_id === 'string'
    && value.voice_library_id.length > 128
    ? '音色库 ID 不能超过 128 个字符。'
    : null;
  const snrTargetError = value.snr_target_db !== null
    && (!Number.isFinite(value.snr_target_db)
      || value.snr_target_db < 0
      || value.snr_target_db > 60)
    ? '目标信噪比必须为自动，或填写 0–60 dB。'
    : null;
  const ambientSourceLabel = ambientSoundPath ? '用户素材' : '内置环境声';
  const ambientDisplayPath = ambientSoundPath ?? '内置环境声';

  return (
    <Form
      className="audio-parameter-controls"
      component="div"
      layout="vertical"
    >
      <section
        aria-labelledby={`${idBase}-feature-heading`}
        className="audio-parameter-controls__section"
      >
        <div className="audio-parameter-controls__section-heading">
          <Typography.Title id={`${idBase}-feature-heading`} level={5}>
            声音来源与自动基线
          </Typography.Title>
          <Typography.Paragraph type="secondary">
            其余声音效果由周期随机化生成，并在下方参数卡中只读展示。
          </Typography.Paragraph>
        </div>

        <div className="audio-parameter-controls__field-grid">
          <Form.Item
            htmlFor={`${idBase}-natural-voice-mode`}
            label="自然动态模式"
          >
            <Select
              aria-label="自然动态模式"
              disabled={disabled}
              id={`${idBase}-natural-voice-mode`}
              options={[
                { value: 'original', label: '保持原声' },
                { value: 'natural_dynamic', label: '自然动态' },
              ]}
              value={value.natural_voice_mode}
              onChange={(next) => onChange('natural_voice_mode', next)}
            />
          </Form.Item>
          <Form.Item
            extra={voiceLibraryError ? null : (
              <span id={voiceLibraryHelpId}>留空时跟随源音色；本地 ID 最多 128 个字符。</span>
            )}
            help={voiceLibraryError ? (
              <span id={voiceLibraryHelpId} role="alert">{voiceLibraryError}</span>
            ) : null}
            htmlFor={`${idBase}-voice-library-id`}
            label="音色库 ID"
            validateStatus={voiceLibraryError ? 'error' : undefined}
          >
            <Input
              allowClear
              aria-describedby={voiceLibraryHelpId}
              aria-invalid={Boolean(voiceLibraryError)}
              aria-label="音色库 ID"
              disabled={disabled}
              id={`${idBase}-voice-library-id`}
              maxLength={128}
              status={voiceLibraryError ? 'error' : undefined}
              value={value.voice_library_id ?? ''}
              onChange={(event) => onChange('voice_library_id', event.target.value || null)}
            />
          </Form.Item>
        </div>

        <div className="audio-parameter-controls__field-grid">
          <Form.Item
            extra={snrTargetError ? null : (
              <span id={snrTargetHelpId}>自动会按源声音基线处理；也可填写 0–60 dB。</span>
            )}
            help={snrTargetError ? (
              <span id={snrTargetHelpId} role="alert">{snrTargetError}</span>
            ) : null}
            htmlFor={`${idBase}-snr-target`}
            label="目标信噪比"
            validateStatus={snrTargetError ? 'error' : undefined}
          >
            <div className="audio-parameter-controls__snr-target">
              <InputNumber
                addonAfter="dB"
                aria-describedby={snrTargetHelpId}
                aria-invalid={Boolean(snrTargetError)}
                aria-label="目标信噪比"
                disabled={disabled}
                id={`${idBase}-snr-target`}
                max={60}
                min={0}
                placeholder="自动"
                status={snrTargetError ? 'error' : undefined}
                step={0.1}
                value={value.snr_target_db}
                onChange={(next) => onChange('snr_target_db', next)}
              />
              <Button
                disabled={disabled || value.snr_target_db === null}
                onClick={() => onChange('snr_target_db', null)}
              >自动</Button>
            </div>
          </Form.Item>

          <Form.Item
            extra={(
              <span id={ambientSoundHelpId}>
                用户素材可选覆盖；未选择时由本地媒体引擎解析内置环境声。
              </span>
            )}
            label="环境声素材"
          >
            <div className="audio-parameter-controls__ambient-source">
              <Input
                aria-describedby={ambientSoundHelpId}
                aria-label="环境声素材路径"
                readOnly
                value={ambientDisplayPath}
              />
              <div className="audio-parameter-controls__ambient-actions">
                <Button
                  aria-label="选择环境声素材"
                  disabled={disabled}
                  icon={<FolderOpenOutlined />}
                  onClick={() => { void onChooseAmbientSound(); }}
                >
                  选择
                </Button>
                <Button
                  aria-label="清除用户环境声素材"
                  disabled={disabled || !ambientSoundPath}
                  icon={<DeleteOutlined />}
                  onClick={onClearAmbientSound}
                >
                  清除
                </Button>
              </div>
              <Typography.Text className="audio-parameter-controls__ambient-status">
                当前来源：<Tag color={ambientSoundPath ? 'processing' : 'default'}>{ambientSourceLabel}</Tag>
              </Typography.Text>
            </div>
          </Form.Item>
        </div>
      </section>
    </Form>
  );
}
