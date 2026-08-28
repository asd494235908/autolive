import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

async function readSource(path) {
  return readFile(new URL(path, import.meta.url), 'utf8');
}

test('插话声音预设参数行保留静态强调色且不再闪烁', async () => {
  const [app, row, layoutCss, parameterCss] = await Promise.all([
    readSource('./App.tsx'),
    readSource('./desktop/interlude-preset-parameter-row.tsx'),
    readSource('./desktop-layout.css'),
    readSource('./media-parameter-panels/media-parameter-panels.css'),
  ]);
  const presetCard = app.slice(
    app.indexOf('title="插话声音预设"'),
    app.indexOf('desktop-media-domain-lane--video'),
  );

  assert.match(app, /import \{ InterludePresetParameterRow \} from '\.\/desktop\/interlude-preset-parameter-row';/);
  assert.match(presetCard, /<InterludePresetParameterRow[\s\S]*label=\{field\.label\}[\s\S]*value=\{formatAudioPresetFieldValue/);
  assert.match(row, /export const InterludePresetParameterRow\s*=\s*memo\(/);
  assert.doesNotMatch(row, /useEffect|useRef|useState|flashGeneration|media-parameter-card--flash/);
  assert.match(layoutCss, /\.desktop-preset-parameter-row\s*\{[^}]*border-left:\s*2px solid #f0bd3e/s);
  assert.doesNotMatch(layoutCss, /\.desktop-preset-parameter-row::after/);
  assert.doesNotMatch(parameterCss, /media-parameter-card--flash|media-parameter-card-flash/);
});
