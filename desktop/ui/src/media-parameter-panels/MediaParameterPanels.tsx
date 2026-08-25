import { useEffect, useId, useRef, useState, type CSSProperties } from 'react';
import { Alert, Card, Progress, Spin, Tag, Typography, theme } from 'antd';

import {
  MEDIA_PARAMETER_DEFINITIONS,
  MEDIA_PARAMETER_STATUS_LABELS,
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
  createMediaParameterValueSignature,
  createRandomParameterAccents,
  type ParameterAccent,
} from './parameter-panel-feedback';
import { normalizeMediaParameterProgress } from './media-parameter-progress';
import './media-parameter-panels.css';

const SECTION_LABELS: Record<MediaParameterSection, string> = {
  video: '普通视频',
  audio: '普通声音',
  advanced: '高级视觉',
};

const PARAMETER_ACCENT_PATHS = (['video', 'advanced', 'audio'] as const).flatMap((section) => (
  MEDIA_PARAMETER_DEFINITIONS[section].map(getMediaParameterPath)
));

const STATUS_COLORS: Record<MediaParameterStatus, string> = {
  implemented: 'success',
  planned: 'warning',
  pending_confirmation: 'default',
};

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
  progressColor,
}: {
  definition: NumericParameterDefinition;
  value: unknown;
  labelId: string;
  progressColor: string;
}) {
  const numericValue = getSafeNumber(value);
  const emptyText = definition.kind === 'readonly-number' ? '尚未测量' : '未设置';
  const valueText = numericValue === null
    ? emptyText
    : `${formatNumber(numericValue)}${definition.unit ? ` ${definition.unit}` : ''}`;

  return (
    <div aria-labelledby={labelId} className="media-parameter-card__numeric-value">
      <Typography.Text className="media-parameter-card__value" strong>
        {valueText}
      </Typography.Text>
      <div className="media-parameter-card__progress">
        <Progress
          aria-label={`${definition.label}：${valueText}；范围 ${definition.min}–${definition.max}${definition.unit ? ` ${definition.unit}` : ''}`}
          percent={normalizeMediaParameterProgress(numericValue, definition.min, definition.max)}
          showInfo={false}
          size={[4, 4]}
          steps={16}
          strokeColor={progressColor}
        />
      </div>
    </div>
  );
}

function BandWeightsValue({
  value,
  labelId,
  definition,
  progressColor,
}: {
  value: unknown;
  labelId: string;
  definition: Extract<MediaParameterDefinition, { kind: 'band-weights' }>;
  progressColor: string;
}) {
  const weights = typeof value === 'object' && value !== null
    ? value as Record<string, unknown>
    : {};

  return (
    <div aria-labelledby={labelId} className="media-parameter-card__bands">
      {VISUAL_BAND_FREQUENCIES_HZ.map((frequencyHz) => {
        const weight = getSafeNumber(weights[String(frequencyHz)]);
        const valueText = weight === null ? '未设置' : `${formatNumber(weight)} ${definition.unit}`;
        return (
          <div className="media-parameter-card__band" key={frequencyHz}>
            <div className="media-parameter-card__band-label">
              <span>{frequencyHz} Hz</span>
              <span>{valueText}</span>
            </div>
            <div className="media-parameter-card__progress">
              <Progress
                aria-label={`${frequencyHz} Hz 权重：${valueText}；范围 ${definition.min}–${definition.max} ${definition.unit}`}
                percent={normalizeMediaParameterProgress(weight, definition.min, definition.max)}
                showInfo={false}
                size={[4, 4]}
                steps={10}
                strokeColor={progressColor}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}

function ParameterCard({
  definition,
  params,
  status,
  idBase,
  progressColor,
  cardPadding,
}: {
  definition: MediaParameterDefinition;
  params: MediaEffectParams;
  status: MediaParameterStatus;
  idBase: string;
  progressColor: string;
  cardPadding: number;
}) {
  const labelId = `${idBase}-${definition.section}-${definition.field}-label`;
  const rawValue = params[definition.section][definition.field];
  const valueSignature = createMediaParameterValueSignature(rawValue);
  const previousValueSignature = useRef(valueSignature);
  const [flashGeneration, setFlashGeneration] = useState(0);

  useEffect(() => {
    if (previousValueSignature.current === valueSignature) return;
    previousValueSignature.current = valueSignature;
    setFlashGeneration((current) => current + 1);
  }, [valueSignature]);

  const baseCardClassName = definition.kind === 'band-weights'
    ? 'media-parameter-card media-parameter-card--band-weights'
    : 'media-parameter-card';
  const cardClassName = flashGeneration === 0
    ? baseCardClassName
    : flashGeneration % 2 === 1
      ? `${baseCardClassName} media-parameter-card--flash-a`
      : `${baseCardClassName} media-parameter-card--flash-b`;
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
          progressColor={progressColor}
          value={rawValue}
        />
      );
      break;
    case 'band-weights':
      valueDisplay = (
        <BandWeightsValue
          definition={definition}
          labelId={labelId}
          progressColor={progressColor}
          value={rawValue}
        />
      );
      break;
    case 'boolean':
      valueDisplay = (
        <Tag color={rawValue === true ? 'success' : 'default'}>
          {getStaticValue(definition, rawValue)}
        </Tag>
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
      className={cardClassName}
      size="small"
      style={cardStyle}
      styles={{ body: { padding: cardPadding } }}
    >
      <div className="media-parameter-card__header">
        <Typography.Text id={labelId} strong>{definition.label}</Typography.Text>
        <Tag className="media-parameter-card__status" color={STATUS_COLORS[status]}>
          {MEDIA_PARAMETER_STATUS_LABELS[status]}
        </Tag>
      </div>
      <div aria-labelledby={labelId} className="media-parameter-card__value-area">
        {valueDisplay}
      </div>
    </Card>
  );
}

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
  return (
    <section aria-labelledby={sectionHeadingId} className="media-parameter-section">
      <Typography.Text
        className="media-parameter-section__title"
        id={sectionHeadingId}
        strong
      >
        {SECTION_LABELS[section]}
      </Typography.Text>
      {groupDefinitions(MEDIA_PARAMETER_DEFINITIONS[section]).map(([group, groupItems]) => {
        const groupHeadingId = `${idBase}-${section}-${group}-heading`;
        return (
          <section aria-labelledby={groupHeadingId} className="media-parameter-section__group" key={group}>
            <Typography.Text
              className="media-parameter-section__group-title"
              id={groupHeadingId}
              strong
            >
              {group}
            </Typography.Text>
            <div className="media-parameter-section__grid">
              {groupItems.map((definition) => {
                const path = getMediaParameterPath(definition);
                const cardAccentColor = REFERENCE_IMAGE_ACCENT_COLORS[parameterAccents[path]];
                return (
                  <ParameterCard
                    cardPadding={cardPadding}
                    definition={definition}
                    idBase={idBase}
                    key={path}
                    params={params}
                    progressColor={cardAccentColor}
                    status={statusOverrides?.[path] ?? definition.status}
                  />
                );
              })}
            </div>
          </section>
        );
      })}
    </section>
  );
}

export function MediaParameterPanels({
  value,
  loading = false,
  error = null,
  statusOverrides,
  audioControls,
}: MediaParameterPanelsProps) {
  const idBase = useId().replace(/:/g, '');
  const [parameterAccents] = useState(() => createRandomParameterAccents(PARAMETER_ACCENT_PATHS));
  const { token } = theme.useToken();

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
      <div aria-label="参数接入状态说明" className="media-parameter-panels__legend">
        {(Object.keys(MEDIA_PARAMETER_STATUS_LABELS) as MediaParameterStatus[]).map((status) => (
          <Tag color={STATUS_COLORS[status]} key={status}>{MEDIA_PARAMETER_STATUS_LABELS[status]}</Tag>
        ))}
      </div>
      <Spin spinning={loading} tip="正在读取媒体参数">
        <div className="media-parameter-panels__columns">
          <div className="media-parameter-panels__lane media-parameter-panels__lane--video">
            <ParameterSection
              cardPadding={token.paddingXXS}
              idBase={idBase}
              parameterAccents={parameterAccents}
              params={value}
              section="video"
              statusOverrides={statusOverrides}
            />
            <ParameterSection
              cardPadding={token.paddingXXS}
              idBase={idBase}
              parameterAccents={parameterAccents}
              params={value}
              section="advanced"
              statusOverrides={statusOverrides}
            />
          </div>
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
        </div>
      </Spin>
    </div>
  );
}
