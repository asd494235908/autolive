import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  commitStagedPaths,
  WINDOWS_PREPARED_FILES,
} from './mpv-runtime-release.mjs';

test('Windows 准备树固定为十一项', () => {
  assert.equal(WINDOWS_PREPARED_FILES.length, 11);
  assert.equal(new Set(WINDOWS_PREPARED_FILES).size, 11);
  assert.equal(WINDOWS_PREPARED_FILES.some((path) => path.includes('build-evidence')), false);
});

test('事务提交预检失败时保留旧输出', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-resource-transaction-'));
  const target = join(root, 'target');
  mkdirSync(target);
  writeFileSync(join(target, 'old.txt'), 'old');

  assert.throws(
    () => commitStagedPaths([{ target, staged: join(root, 'missing-stage') }]),
    /staging|暂存/i,
  );
  assert.equal(readFileSync(join(target, 'old.txt'), 'utf8'), 'old');
});

test('事务提交用完整 staging 替换旧目录', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-resource-transaction-'));
  const target = join(root, 'target');
  const staged = join(root, 'staged');
  mkdirSync(target);
  mkdirSync(staged);
  writeFileSync(join(target, 'old.txt'), 'old');
  writeFileSync(join(staged, 'new.txt'), 'new');

  commitStagedPaths([{ target, staged }]);

  assert.equal(readFileSync(join(target, 'new.txt'), 'utf8'), 'new');
  assert.throws(() => readFileSync(join(target, 'old.txt'), 'utf8'));
});

test('多目标提交中途失败会回滚已经替换的旧输出', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-resource-transaction-'));
  const firstTarget = join(root, 'first-target');
  const firstStage = join(root, 'first-stage');
  const secondStage = join(root, 'second-stage');
  const blockedParent = join(root, 'not-a-directory');
  mkdirSync(firstTarget);
  mkdirSync(firstStage);
  mkdirSync(secondStage);
  writeFileSync(join(firstTarget, 'old.txt'), 'old');
  writeFileSync(join(firstStage, 'new.txt'), 'new');
  writeFileSync(join(secondStage, 'new.txt'), 'new');
  writeFileSync(blockedParent, 'file');

  assert.throws(() => commitStagedPaths([
    { target: firstTarget, staged: firstStage },
    { target: join(blockedParent, 'second-target'), staged: secondStage },
  ]));

  assert.equal(readFileSync(join(firstTarget, 'old.txt'), 'utf8'), 'old');
  assert.throws(() => readFileSync(join(firstTarget, 'new.txt'), 'utf8'));
});

test('事务提交拒绝相互嵌套的目标与 staging 路径', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-resource-transaction-'));
  const target = join(root, 'target');
  const staged = join(target, 'staged');
  mkdirSync(staged, { recursive: true });
  writeFileSync(join(staged, 'new.txt'), 'new');

  assert.throws(
    () => commitStagedPaths([{ target, staged }]),
    /相同或嵌套/,
  );
  assert.equal(readFileSync(join(staged, 'new.txt'), 'utf8'), 'new');
});
