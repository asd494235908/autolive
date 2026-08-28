import { memo, useId, useState, type CSSProperties } from 'react';
import { Alert, Card, Spin, Tag, Typography, theme } from 'antd';

import {
  MEDIA_PARAMETER_DEFINITIONS,
  VISUAL_BAND_FREQUENCIES_HZ,
  getMediaParameterPath,
  type MediaParameterDefinition,
  type NumericParameterDefinition,
} from './parameter-definitions';
import type {
  MediaEffectParams,
  MediaParameterPanelsProps,
  MediaParameterSection,
  MediaParameterStatus,
} from './media-parameter-types';
import {
  REFERENCE_IMAGE_ACCENT_COLORS,
  createRandomParameterAccents,
  type ParameterAccent,
} from './parameter-panel-feedback';
import './media-parameter-panels.css';

const SECTION_LABELS: Record<MediaParameterSection, string> = {
  video: '普通视频',
  audio: '普通声音',
  advanced: '高级视觉',
};

const PARAMETER_ACCENT_PATHS = (['video', 'advanced', 'audio'] as const).flatMap((section) => (
  MEDIA_PARAMETER_DEFINITIONS[section].map(getMediaParameterPath)
));

const numberFormatter = new Intl.NumberFormat('zh-CN', {
  maximumFractionDigits: 3,
});

function getSafeNumber(value: unknown) {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function formatNumber(value: number) {
  return numberFormatter.format(value);
}

function getStaticValue(definition: MediaParameterDefinition, value: unknown) {
  switch (definition.kind) {
    case 'boolean':
      return value === true ? '开启' : '关闭';
    case 'select':
      return definition.options.find((option) => Object.is(option.value, value))?.label ?? '未设置';
    case 'text':
      return typeof value === 'string' && value ? value : '跟随源素材';
    default:
      return '未设置';
  }
}

function NumericValue({
  definition,
  value,
  labelId,
}: {
  definition: NumericParameterDefinition;
  value: unknown;
  labelId: string;
}) {
  const numericValue = getSafeNumber(value);
  const emptyText = definition.kind === 'readonly-number' ? '尚未测量' : '未设置';
  const valueText = numericValue === null
    ? emptyText
    : `${formatNumber(numericValue)}${definition.unit ? ` ${definition.unit}` : ''}`;

  return (
    <Typography.Text aria-labelledby={labelId} className="media-parameter-card__value" strong>
      {valueText}
    </Typography.Text>
  );
}

const BandWeightCards = memo(function BandWeightCards({
  value,
  definition,
  progressColor,
  status,
}: {
  value: unknown;
  definition: Extract<MediaParameterDefinition, { kind: 'band-weights' }>;
  progressColor: string;
  status: MediaParameterStatus;
}) {
  const weights = typeof value === 'object' && value !== null
    ? value as Record<string, unknown>
    : {};

  return (
    <>
      {VISUAL_BAND_FREQUENCIES_HZ.map((frequencyHz) => {
        const weight = getSafeNumber(weights[String(frequencyHz)]);
        const valueText = weight === null ? '未设置' : `${formatNumber(weight)} ${definition.unit}`;
        return (
          <Card
            className="media-parameter-card media-parameter-card--band"
            key={frequencyHz}
            size="small"
            style={{ '--media-parameter-accent': progressColor, borderInlineStartColor: progressColor } as CSSProperties}
            styles={{ body: { padding: 4 } }}
          >
            <div className="media-parameter-card__header">
              <Typography.Text>{frequencyHz} Hz 权重</Typography.Text>
              <div className="media-parameter-card__result">
                <Typography.Text className="media-parameter-card__value" strong>{valueText}</Typography.Text>
                {status === 'implemented' ? null : <Tag color={status === 'planned' ? 'warning' : 'default'}>{status === 'planned' ? '正式需求·待实现' : '待确认'}</Tag>}
              </div>
            </div>
          </Card>
        );
      })}
    </>
  );
});

const MediaParameterCard = memo(function MediaParameterCard({
  definition,
  rawValue,
  status,
  idBase,
  progressColor,
  cardPadding,
}: {
  definition: MediaParameterDefinition;
  rawValue: unknown;
  status: MediaParameterStatus;
  idBase: string;
  progressColor: string;
  cardPadding: number;
}) {
  const labelId = `${idBase}-${definition.section}-${definition.field}-label`;
  const cardStyle: CSSProperties & { '--media-parameter-accent': string } = {
    '--media-parameter-accent': progressColor,
    borderInlineStartColor: progressColor,
  };

  let valueDisplay;
  switch (definition.kind) {
    case 'number':
    case 'optional-number':
    case 'readonly-number':
      valueDisplay = (
        <NumericValue
          definition={definition}
          labelId={labelId}
          value={rawValue}
        />
      );
      break;
    case 'band-weights': valueDisplay = null; break;
    case 'boolean':
      valueDisplay = (
        <Typography.Text className="media-parameter-card__value" strong>
          {getStaticValue(definition, rawValue)}
        </Typography.Text>
      );
      break;
    case 'select':
    case 'text':
      valueDisplay = (
        <Typography.Text className="media-parameter-card__value" strong>
          {getStaticValue(definition, rawValue)}
        </Typography.Text>
      );
      break;
  }

  return (
    <Card
      className="media-parameter-card"
      size="small"
      style={cardStyle}
      styles={{ body: { padding: cardPadding } }}
    >
      <div className="media-parameter-card__header">
        <Typography.Text id={labelId} strong>{definition.label}</Typography.Text>
        <div aria-labelledby={labelId} className="media-parameter-card__result">
          {valueDisplay}
          {status === 'implemented' ? null : (
            <Tag className="media-parameter-card__status" color={status === 'planned' ? 'warning' : 'default'}>
              {status === 'planned' ? '正式需求·待实现' : '待确认'}
            </Tag>
          )}
        </div>
      </div>
    </Card>
  );
});

function groupDefinitions(definitions: readonly MediaParameterDefinition[]) {
  const groups = new Map<string, MediaParameterDefinition[]>();
  definitions.forEach((definition) => {
    const group = groups.get(definition.group) ?? [];
    group.push(definition);
    groups.set(definition.group, group);
  });
  return [...groups.entries()];
}

function ParameterSection({
  section,
  params,
  statusOverrides,
  idBase,
  parameterAccents,
  cardPadding,
}: {
  section: MediaParameterSection;
  params: MediaEffectParams;
  statusOverrides: MediaParameterPanelsProps['statusOverrides'];
  idBase: string;
  parameterAccents: Record<string, ParameterAccent>;
  cardPadding: number;
}) {
  const sectionHeadingId = `${idBase}-${section}-heading`;
  const groupedDefinitions = groupDefinitions(MEDIA_PARAMETER_DEFINITIONS[section]);
  const visibleParameterCount = MEDIA_PARAMETER_DEFINITIONS[section].reduce(
    (count, definition) => count + (definition.kind === 'band-weights' ? VISUAL_BAND_FREQUENCIES_HZ.length : 1),
    0,
  );
  return (
    <section aria-labelledby={sectionHeadingId} className="media-parameter-section">
      <div className="media-parameter-section__heading">
        <Typography.Text
          className="media-parameter-section__title"
          id={sectionHeadingId}
          strong
        >
          {SECTION_LABELS[section]}
        </Typography.Text>
        <Typography.Text className="media-parameter-section__summary">
          {visibleParameterCount} 项 · 只读快照
        </Typography.Text>
      </div>
      <div className="media-parameter-section__grid">
        {groupedDefinitions.flatMap(([, groupItems]) => (
          groupItems.map((definition) => {
            const path = getMediaParameterPath(definition);
            const cardAccentColor = REFERENCE_IMAGE_ACCENT_COLORS[parameterAccents[path]];
            const status = statusOverrides?.[path] ?? definition.status;
            if (definition.kind === 'band-weights') {
              return (
                <BandWeightCards
                  definition={definition}
                  key={path}
                  progressColor={cardAccentColor}
                  status={status}
                  value={params[definition.section][definition.field]}
                />
              );
            }
            return (
              <MediaParameterCard
                cardPadding={cardPadding}
                definition={definition}
                idBase={idBase}
                key={path}
                progressColor={cardAccentColor}
                rawValue={params[definition.section][definition.field]}
                status={status}
              />
            );
          })
        ))}
      </div>
    </section>
  );
}

export const MediaParameterPanels = memo(function MediaParameterPanels({
  value,
  loading = false,
  error = null,
  statusOverrides,
  audioControls,
  sections = ['video', 'advanced', 'audio'],
}: MediaParameterPanelsProps) {
  const idBase = useId().replace(/:/g, '');
  const [parameterAccents] = useState(() => createRandomParameterAccents(PARAMETER_ACCENT_PATHS));
  const { token } = theme.useToken();
  const hasVideoLane = sections.some((section) => section === 'video' || section === 'advanced');
  const hasAudioLane = sections.includes('audio');

  return (
    <div aria-busy={loading} className="media-parameter-panels">
      {error ? (
        <Alert
          className="media-parameter-panels__alert"
          description={error}
          message="媒体参数读取失败"
          role="alert"
          showIcon
          type="error"
        />
      ) : null}
      <Spin spinning={loading} tip="正在读取媒体参数">
        <div className={`media-parameter-panels__columns${hasVideoLane && hasAudioLane ? ' media-parameter-panels__columns--split' : ''}`}>
          {hasVideoLane ? (
            <div className="media-parameter-panels__lane media-parameter-panels__lane--video">
              {sections.filter((section) => section === 'video' || section === 'advanced').map((section) => (
                <ParameterSection
                  cardPadding={token.paddingXXS}
                  idBase={idBase}
                  key={section}
                  parameterAccents={parameterAccents}
                  params={value}
                  section={section}
                  statusOverrides={statusOverrides}
                />
              ))}
            </div>
          ) : null}
          {hasAudioLane ? (
            <div className="media-parameter-panels__lane media-parameter-panels__lane--audio">
              {audioControls ? (
                <div className="media-parameter-panels__audio-controls">
                  {audioControls}
                </div>
              ) : null}
              <ParameterSection
                cardPadding={token.paddingXXS}
                idBase={idBase}
                parameterAccents={parameterAccents}
                params={value}
                section="audio"
                statusOverrides={statusOverrides}
              />
            </div>
          ) : null}
        </div>
      </Spin>
    </div>
  );
});
