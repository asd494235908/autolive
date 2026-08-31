import assert from 'node:assert/strict';
import test from 'node:test';

import {
  capturePausedScenarioFrames,
  compareRawRgbPairs,
  compareRawRgbScenarios,
  serializeShaderOptions,
} from './mpv-frame-difference.mjs';

test('shader 快照按字段稳定排序并拒绝命令注入与非有限数', () => {
  assert.equal(serializeShaderOptions({ al_z: 2, al_a: 1 }), 'al_a=1,al_z=2');
  assert.throws(() => serializeShaderOptions({ command: 1 }), /不安全/);
  assert.throws(() => serializeShaderOptions({ al_bad: Number.NaN }), /有限数/);
});

test('同一帧 RGB 成对比较按像素差异 fail-closed', () => {
  const unchanged = Buffer.from([10, 20, 30, 10, 20, 30]);
  const changed = Buffer.from([10, 20, 30, 20, 30, 40]);
  const raw = Buffer.concat([unchanged, changed]);
  const [result] = compareRawRgbPairs(raw, ['al_test'], {
    width: 2,
    height: 1,
    minimumChangedPixelRatio: 0.5,
    minimumMeanAbsoluteDelta: 1,
  });
  assert.equal(result.status, 'passed');
  assert.equal(result.changedPixels, 1);
  assert.equal(result.changedPixelRatio, 0.5);
  assert.equal(result.maximumChannelDelta, 10);

  const [same] = compareRawRgbPairs(Buffer.concat([unchanged, unchanged]), ['al_test'], {
    width: 2,
    height: 1,
  });
  assert.equal(same.status, 'failed');
});

test('RGB 帧数量必须与 neutral/active 场景严格匹配', () => {
  assert.throws(
    () => compareRawRgbPairs(Buffer.alloc(5), ['al_test'], { width: 1, height: 1 }),
    /不匹配/,
  );
});

test('捕获器按场景声明捕获三档或两档并在证据中保留 mode', async () => {
  const commands = [];
  const ipc = {
    async send(command) {
      commands.push(command);
      if (command[0] !== 'get_property') return { data: null };
      return { data: command[1] === 'pause' ? false : 12.5 };
    },
  };
  const options = (value) => ({ al_test: value });
  const captures = await capturePausedScenarioFrames({
    ipc,
    directory: 'E:/fake-frame-difference',
    scenarios: [
      {
        field: 'al_product',
        mode: 'product_progression',
        states: [
          { role: 'baseline', options: options(0) },
          { role: 'low', options: options(1) },
          { role: 'active', options: options(2) },
        ],
      },
      {
        field: 'seed_isolation',
        mode: 'expected_same',
        states: [
          { role: 'seed_zero', options: options(0) },
          { role: 'seed_changed', options: options(1) },
        ],
      },
    ],
  });

  assert.deepEqual(captures.map(({ field, mode, role }) => ({ field, mode, role })), [
    { field: 'al_product', mode: 'product_progression', role: 'baseline' },
    { field: 'al_product', mode: 'product_progression', role: 'low' },
    { field: 'al_product', mode: 'product_progression', role: 'active' },
    { field: 'seed_isolation', mode: 'expected_same', role: 'seed_zero' },
    { field: 'seed_isolation', mode: 'expected_same', role: 'seed_changed' },
  ]);
  assert.equal(commands.filter((command) => command[0] === 'screenshot-to-file').length, 5);
  assert.ok(captures.every(({ mediaPtsSeconds }) => mediaPtsSeconds === 12.5));
  assert.deepEqual(commands.at(-1), ['set_property', 'pause', false]);
});

test('捕获器拒绝暂停期间 PTS 漂移，不能把跨帧差异当作参数效果', async () => {
  let ptsReadCount = 0;
  const ipc = {
    async send(command) {
      if (command[0] !== 'get_property') return { data: null };
      if (command[1] === 'pause') return { data: false };
      ptsReadCount += 1;
      return { data: ptsReadCount === 1 ? 8 : 8.04 };
    },
  };
  await assert.rejects(
    capturePausedScenarioFrames({
      ipc,
      directory: 'E:/fake-frame-difference-drift',
      scenarios: [{
        field: 'al_product',
        neutralOptions: { al_product: 0 },
        activeOptions: { al_product: 1 },
      }],
    }),
    /截图跨帧/,
  );
});

test('三种帧差模式分别执行递进、同帧一致和明显差异门禁', () => {
  const captures = [
    { scenarioIndex: 0, field: 'al_product', mode: 'product_progression', role: 'baseline' },
    { scenarioIndex: 0, field: 'al_product', mode: 'product_progression', role: 'low' },
    { scenarioIndex: 0, field: 'al_product', mode: 'product_progression', role: 'active' },
    { scenarioIndex: 1, field: 'seed_isolation', mode: 'expected_same', role: 'seed_zero' },
    { scenarioIndex: 1, field: 'seed_isolation', mode: 'expected_same', role: 'seed_changed' },
    { scenarioIndex: 2, field: 'runtime_gate', mode: 'pair_difference', role: 'neutral' },
    { scenarioIndex: 2, field: 'runtime_gate', mode: 'pair_difference', role: 'active' },
  ];
  const raw = Buffer.from([
    10, 10, 10,
    12, 10, 10,
    20, 10, 10,
    30, 30, 30,
    30, 30, 30,
    40, 40, 40,
    50, 40, 40,
  ]);
  const results = compareRawRgbScenarios(raw, captures, {
    width: 1,
    height: 1,
    minimumChangedPixelRatio: 1,
    minimumMeanAbsoluteDelta: 0.5,
  });
  assert.deepEqual(results.map(({ mode, status }) => ({ mode, status })), [
    { mode: 'product_progression', status: 'passed' },
    { mode: 'expected_same', status: 'passed' },
    { mode: 'pair_difference', status: 'passed' },
  ]);
  assert.equal(results[0].lowBelowActive, true);
});

test('产品 low 可证明一档 code-value 响应，但 active 仍执行原明显差异阈值', () => {
  const captures = ['baseline', 'low', 'active'].map((role) => ({
    scenarioIndex: 0, field: 'al_low_sensitivity', mode: 'product_progression', role,
  }));
  const baseline = Buffer.alloc(100 * 3, 10);
  const low = Buffer.from(baseline);
  const active = Buffer.from(baseline);
  for (let pixel = 0; pixel < 100; pixel += 1) {
    low[pixel * 3] += 1;
    active[pixel * 3] += 3;
  }
  const [passed] = compareRawRgbScenarios(
    Buffer.concat([baseline, low, active]), captures, { width: 100, height: 1 },
  );
  assert.equal(passed.status, 'passed');
  assert.equal(passed.baselineToLow.changedPixels, 0);
  assert.equal(passed.baselineToLow.oneCodeValueChangedPixels, 100);

  const activeOnlyOneCodeValue = Buffer.from(baseline);
  for (let pixel = 0; pixel < 100; pixel += 1) activeOnlyOneCodeValue[pixel * 3] += 1;
  const [failed] = compareRawRgbScenarios(
    Buffer.concat([baseline, low, activeOnlyOneCodeValue]), captures, { width: 100, height: 1 },
  );
  assert.equal(failed.status, 'failed');
  assert.equal(failed.baselineToActive.changedPixels, 0);
});

test('产品开关只比较 baseline/active，非强度参数不要求 active 均值大于 low', () => {
  const switchCaptures = ['baseline', 'active'].map((role) => ({
    scenarioIndex: 0, field: 'al_switch', mode: 'product_switch', role,
  }));
  const [productSwitch] = compareRawRgbScenarios(
    Buffer.from([10, 10, 10, 12, 10, 10]), switchCaptures,
    { width: 1, height: 1, minimumChangedPixelRatio: 1, minimumMeanAbsoluteDelta: 0.5 },
  );
  assert.equal(productSwitch.status, 'passed');

  const responseCaptures = ['baseline', 'low', 'active'].map((role) => ({
    scenarioIndex: 0, field: 'al_frequency', mode: 'product_response', role,
  }));
  const [response] = compareRawRgbScenarios(
    Buffer.from([10, 10, 10, 15, 10, 10, 13, 10, 10]), responseCaptures,
    { width: 1, height: 1, minimumChangedPixelRatio: 1, minimumMeanAbsoluteDelta: 0.5 },
  );
  assert.equal(response.status, 'passed');
  assert.equal(response.lowBelowActive, false);
  assert.equal(response.responseSeparated, true);
});

test('非单调产品响应的 low 与 active 相同必须 fail-closed', () => {
  const captures = ['baseline', 'low', 'active'].map((role) => ({
    scenarioIndex: 0, field: 'al_frequency', mode: 'product_response', role,
  }));
  const [response] = compareRawRgbScenarios(
    Buffer.from([10, 10, 10, 15, 10, 10, 15, 10, 10]), captures,
    { width: 1, height: 1, minimumChangedPixelRatio: 1, minimumMeanAbsoluteDelta: 0.5 },
  );

  assert.equal(response.status, 'failed');
  assert.equal(response.responseSeparated, false);
  assert.equal(response.lowToActive.changedPixels, 0);
});

test('产品 low 不低于 active、expected_same 超阈值均 fail-closed', () => {
  const productCaptures = ['baseline', 'low', 'active'].map((role) => ({
    scenarioIndex: 0, field: 'al_product', mode: 'product_progression', role,
  }));
  const [product] = compareRawRgbScenarios(
    Buffer.from([10, 10, 10, 20, 10, 10, 20, 10, 10]),
    productCaptures,
    { width: 1, height: 1, minimumChangedPixelRatio: 1, minimumMeanAbsoluteDelta: 0.5 },
  );
  assert.equal(product.status, 'failed');
  assert.equal(product.lowBelowActive, false);

  const sameCaptures = ['seed_zero', 'seed_changed'].map((role) => ({
    scenarioIndex: 0, field: 'seed_isolation', mode: 'expected_same', role,
  }));
  const [same] = compareRawRgbScenarios(
    Buffer.from([10, 10, 10, 11, 10, 10]),
    sameCaptures,
    { width: 1, height: 1 },
  );
  assert.equal(same.status, 'failed');
  assert.equal(same.maximumChannelDelta, 1);

  const sparseReference = Buffer.alloc(100 * 3, 10);
  const sparseWithinOne = Buffer.from(sparseReference);
  sparseWithinOne[0] = 11;
  const [withinTolerance] = compareRawRgbScenarios(
    Buffer.concat([sparseReference, sparseWithinOne]),
    sameCaptures,
    { width: 100, height: 1 },
  );
  assert.equal(withinTolerance.status, 'passed');

  const sparseOverOne = Buffer.from(sparseReference);
  sparseOverOne[0] = 12;
  const [overChannelLimit] = compareRawRgbScenarios(
    Buffer.concat([sparseReference, sparseOverOne]),
    sameCaptures,
    { width: 100, height: 1 },
  );
  assert.equal(overChannelLimit.meanAbsoluteDelta < 0.01, true);
  assert.equal(overChannelLimit.maximumChannelDelta, 2);
  assert.equal(overChannelLimit.status, 'failed');
});
