import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

async function loadModule(exports) {
  const source = await readFile(new URL('./fixedSpeech.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  const module = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
  return Object.fromEntries(exports.map((name) => [name, module[name]]));
}

test('固定话术文本按 Unicode 字符限制为 1 到 500 字', async () => {
  const { getFixedSpeechTextError } = await loadModule(['getFixedSpeechTextError']);

  assert.equal(getFixedSpeechTextError('  你好  '), null);
  assert.equal(getFixedSpeechTextError('   '), '文本不能为空');
  assert.equal(getFixedSpeechTextError('你'.repeat(500)), null);
  assert.equal(getFixedSpeechTextError('你'.repeat(501)), '文本最多 500 个字符');
  assert.equal(getFixedSpeechTextError('😀'.repeat(500)), null);
});

test('跨窗口固定话术命令拒绝无效版本、空操作 ID 和超长文本', async () => {
  const { isFixedSpeechCommandMessage } = await loadModule(['isFixedSpeechCommandMessage']);
  const valid = {
    version: 1,
    type: 'fixed-speech-command',
    action: 'speak',
    operation_id: 'speech-1',
    text: '欢迎来到直播间',
  };

  assert.equal(isFixedSpeechCommandMessage(valid), true);
  assert.equal(isFixedSpeechCommandMessage({ ...valid, version: 2 }), false);
  assert.equal(isFixedSpeechCommandMessage({ ...valid, operation_id: ' ' }), false);
  assert.equal(isFixedSpeechCommandMessage({ ...valid, text: '你'.repeat(501) }), false);
  assert.equal(isFixedSpeechCommandMessage({
    version: 1,
    type: 'fixed-speech-command',
    action: 'cancel',
    operation_id: 'speech-1',
  }), true);
});

test('跨窗口状态只接受受支持状态和有界错误信息', async () => {
  const { isFixedSpeechStatusMessage } = await loadModule(['isFixedSpeechStatusMessage']);
  const valid = {
    version: 1,
    type: 'fixed-speech-status',
    operation_id: 'speech-1',
    status: 'playing',
    error: null,
  };

  assert.equal(isFixedSpeechStatusMessage(valid), true);
  assert.equal(isFixedSpeechStatusMessage({ ...valid, status: 'unknown' }), false);
  assert.equal(isFixedSpeechStatusMessage({ ...valid, error: '错'.repeat(501) }), false);
});

test('语音选择只使用本地语音并优先中文', async () => {
  const { selectLocalSpeechVoice } = await loadModule(['selectLocalSpeechVoice']);
  const onlineChinese = { name: 'Online zh', lang: 'zh-CN', localService: false, default: true };
  const localEnglish = { name: 'Local en', lang: 'en-US', localService: true, default: true };
  const localChinese = { name: 'Local zh', lang: 'zh-CN', localService: true, default: false };

  assert.equal(selectLocalSpeechVoice([onlineChinese, localEnglish, localChinese]), localChinese);
  assert.equal(selectLocalSpeechVoice([onlineChinese, localEnglish]), localEnglish);
  assert.equal(selectLocalSpeechVoice([onlineChinese]), null);
});
