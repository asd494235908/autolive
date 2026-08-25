import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

async function loadResponseHelpers() {
  const source = await readFile(new URL('./controlPlaneClient.ts', import.meta.url), 'utf8');
  const start = source.indexOf('export class ControlPlaneError');
  const end = source.indexOf('function createRequestId()', start);
  assert.ok(start >= 0 && end > start);
  const helperSource = source
    .slice(start, end)
    .replace('async function readBoundedResponseText', 'export async function readBoundedResponseText')
    .replace('async function parseJsonSafely', 'export async function parseJsonSafely');
  const compiled = ts.transpileModule(helperSource, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

test('桌面端控制面请求遵守 JSON、请求追踪和统一错误契约', async () => {
  const source = await readFile(new URL('./controlPlaneClient.ts', import.meta.url), 'utf8');
  const packageJson = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));

  assert.match(source, /from '\.\/api\/openapi\.generated'/);
  assert.match(source, /type OpenAPISchemas = OpenAPIComponents\['schemas'\]/);
  assert.match(source, /export type ErrorResponseDto = OpenAPISchemas\['ErrorResponse'\]/);
  assert.match(source, /export type DeviceSummaryDto = OpenAPISchemas\['DeviceSummary'\]/);
  assert.match(source, /export type DirectLLMCallRecordRequestDto = OpenAPISchemas\['DirectLLMCallRecordRequest'\]/);
  assert.equal(packageJson.scripts['api:generate'], 'openapi-typescript ../../接口契约/openapi.yaml -o src/api/openapi.generated.ts');
  assert.equal(packageJson.scripts['api:check'], 'openapi-typescript ../../接口契约/openapi.yaml -o src/api/openapi.generated.ts --check');
  assert.match(source, /Accept: 'application\/json'/);
  assert.match(source, /'X-Request-Id': requestId/);
  assert.match(source, /Content-Type.*application\/json/);
  assert.match(source, /details\?: ApiErrorDetailDto\[\]/);
  assert.match(source, /code: 'NETWORK_ERROR'/);
  assert.match(source, /code: 'CONTROL_PLANE_TIMEOUT'/);
  assert.match(source, /requestId: errorPayload\.request_id \?\? requestId/);
  assert.match(source, /allowAuditRetry = true/);
  assert.match(source, /response\.status === 503/);
  assert.match(source, /payload\.code === 'AUDIT_UNAVAILABLE'/);
  assert.match(source, /requestJson<T>\(path, init, allowAuthRefresh, false\)/);
  assert.match(source, /VITE_CONTROL_PLANE_BASE_URL/);
  assert.match(source, /VITE_CONTROL_PLANE_ENV/);
  assert.match(source, /VITE_CONTROL_PLANE_ENV === 'test'/);
  assert.match(source, /loginControlPlane\(request: LoginRequestDto\).*'\/api\/v1\/auth\/login'/);
  assert.match(source, /type ActivateDeviceRequestDto = Pick<OpenAPISchemas\['ActivateDeviceRequest'\], 'device'>/);
  assert.match(source, /activateDeviceControlPlane\(accessToken: string, request: ActivateDeviceRequestDto\)/);
  assert.match(source, /http:\/\/101\.96\.208\.132:9090/);
  assert.match(source, /must be set for production desktop builds/);
  assert.match(source, /parsed\.protocol !== 'https:'/);
  assert.match(source, /must use HTTPS for production desktop builds/);
  assert.match(source, /must not contain credentials, query, or hash/);
  assert.doesNotMatch(source, /192\.168\.100\.213/);
});

test('控制面响应体按实际字节限制为 1MiB', async () => {
  const source = await readFile(new URL('./controlPlaneClient.ts', import.meta.url), 'utf8');

  assert.match(source, /CONTROL_PLANE_MAX_RESPONSE_BYTES\s*=\s*1024\s*\*\s*1024/);
  assert.match(source, /headers\.get\(['"]content-length['"]\)/i);
  assert.match(source, /body\.getReader\(\)/);
  assert.match(source, /new TextDecoder\(/);
  assert.match(source, /totalBytes\s*\+=\s*value\.byteLength/);
  assert.match(source, /await reader\.cancel\(\)/);
  assert.match(source, /code:\s*'CONTROL_PLANE_RESPONSE_TOO_LARGE'/);
  assert.match(source, /status:\s*502/);
  assert.match(source, /requestId/);
  assert.doesNotMatch(source, /response\.text\(\)/);
});

test('控制面有界读取兼容空响应、非法 JSON 和跨块 UTF-8', async () => {
  const { parseJsonSafely, readBoundedResponseText } = await loadResponseHelpers();
  assert.equal(await parseJsonSafely(new Response(null), 'request-empty'), null);
  assert.equal(await parseJsonSafely(new Response('not-json'), 'request-text'), 'not-json');

  const encoded = new TextEncoder().encode('{"message":"你好🙂"}');
  const response = new Response(new ReadableStream({
    start(controller) {
      for (const byte of encoded) controller.enqueue(Uint8Array.of(byte));
      controller.close();
    },
  }));
  assert.deepEqual(await parseJsonSafely(response, 'request-json'), { message: '你好🙂' });

  const exactLimit = new Uint8Array(1024 * 1024).fill('a'.charCodeAt(0));
  assert.equal(
    (await readBoundedResponseText(new Response(exactLimit), 'request-limit')).length,
    1024 * 1024,
  );
});

test('控制面有界读取提前拒绝声明超限并按实际流量拦截错误长度头', async () => {
  const { ControlPlaneError, readBoundedResponseText } = await loadResponseHelpers();
  const verifyTooLarge = (requestId) => (error) => (
    error instanceof ControlPlaneError
    && error.code === 'CONTROL_PLANE_RESPONSE_TOO_LARGE'
    && error.status === 502
    && error.requestId === requestId
  );

  await assert.rejects(
    readBoundedResponseText(new Response(null, {
      headers: { 'content-length': String(1024 * 1024 + 1) },
    }), 'request-declared'),
    verifyTooLarge('request-declared'),
  );

  let cancelled = false;
  const oversizedStream = new ReadableStream({
    pull(controller) {
      controller.enqueue(new Uint8Array(600 * 1024));
    },
    cancel() {
      cancelled = true;
    },
  });
  await assert.rejects(
    readBoundedResponseText(new Response(oversizedStream, {
      headers: { 'content-length': 'not-a-number' },
    }), 'request-streamed'),
    verifyTooLarge('request-streamed'),
  );
  assert.equal(cancelled, true);
});
