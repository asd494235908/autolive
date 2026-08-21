import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

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
  assert.match(source, /http:\/\/101\.96\.208\.132:9090/);
  assert.match(source, /must be set for production desktop builds/);
  assert.match(source, /parsed\.protocol !== 'https:'/);
  assert.match(source, /must use HTTPS for production desktop builds/);
  assert.match(source, /must not contain credentials, query, or hash/);
  assert.doesNotMatch(source, /192\.168\.100\.213/);
});
