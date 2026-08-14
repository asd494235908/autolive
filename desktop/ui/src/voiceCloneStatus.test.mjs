import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

async function loadTypeScriptModule(fileName, exports) {
  const source = await readFile(new URL(`./${fileName}`, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  const module = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
  return Object.fromEntries(exports.map((name) => [name, module[name]]));
}

test('preparation waits for the source MP4 hash and tells the user why', async () => {
  const { getVoiceClonePrepareDisabledReason, getVoiceCloneIdleNotice } = await loadTypeScriptModule(
    'voiceCloneStatus.ts',
    ['getVoiceClonePrepareDisabledReason', 'getVoiceCloneIdleNotice'],
  );

  assert.equal(
    getVoiceClonePrepareDisabledReason({
      hasSource: true,
      sourceHashStatus: 'pending',
      workerAvailable: true,
      workerReason: null,
      status: 'idle',
    }),
    '正在计算当前 MP4 的完整 SHA-256，请稍候',
  );
  assert.equal(
    getVoiceCloneIdleNotice('pending'),
    '正在计算当前 MP4 的完整 SHA-256，完成后即可准备人声。',
  );
  assert.equal(
    getVoiceCloneIdleNotice('ready'),
    '人声模型和运行环境由安装包提供，首次使用会加载模型，请稍候。',
  );
  assert.equal(
    getVoiceClonePrepareDisabledReason({
      hasSource: true,
      sourceHashStatus: 'ready',
      workerAvailable: true,
      workerReason: null,
      status: 'idle',
    }),
    null,
  );
});
