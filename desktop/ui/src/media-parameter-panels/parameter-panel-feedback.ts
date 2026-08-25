export const PARAMETER_ACCENT_KEYS = [
  'cyan',
  'orange',
  'purple',
  'magenta',
  'yellow',
] as const;

export type ParameterAccent = typeof PARAMETER_ACCENT_KEYS[number];

// 参考图高饱和强调色的量化近似值，仅供媒体参数面板使用。
export const REFERENCE_IMAGE_ACCENT_COLORS: Readonly<Record<ParameterAccent, string>> = {
  cyan: '#38B8F8',
  purple: '#C888F8',
  magenta: '#E878F8',
  orange: '#F89838',
  yellow: '#F8C818',
};

function takeRandomAccent(pool: ParameterAccent[], random: () => number) {
  const index = Math.min(pool.length - 1, Math.max(0, Math.floor(random() * pool.length)));
  return pool.splice(index, 1)[0];
}

export function createRandomParameterAccents(
  paths: readonly string[],
  random: () => number = Math.random,
): Record<string, ParameterAccent> {
  const assignments: Record<string, ParameterAccent> = {};
  let pool: ParameterAccent[] = [];
  let previousAccent: ParameterAccent | undefined;

  paths.forEach((path) => {
    let deferredAccent: ParameterAccent | undefined;
    if (pool.length === 0) {
      pool = [...PARAMETER_ACCENT_KEYS];
      if (previousAccent) {
        [deferredAccent] = pool.splice(pool.indexOf(previousAccent), 1);
      }
    }

    const accent = takeRandomAccent(pool, random);
    if (deferredAccent) pool.push(deferredAccent);
    assignments[path] = accent;
    previousAccent = accent;
  });

  return assignments;
}

export function createMediaParameterValueSignature(value: unknown): string {
  if (value === null) return 'null';
  if (typeof value === 'number') {
    if (Number.isNaN(value)) return 'number:NaN';
    return `number:${Object.is(value, -0) ? '-0' : value}`;
  }
  if (typeof value !== 'object') return `${typeof value}:${String(value)}`;
  if (Array.isArray(value)) {
    return `array:[${value.map(createMediaParameterValueSignature).join(',')}]`;
  }

  const entries = Object.entries(value as Record<string, unknown>)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, entryValue]) => `${key}:${createMediaParameterValueSignature(entryValue)}`);
  return `object:{${entries.join(',')}}`;
}
