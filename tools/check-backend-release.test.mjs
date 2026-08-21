import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { checkBackendRelease } from './check-backend-release.mjs';

const root = resolve(fileURLToPath(new URL('..', import.meta.url)));
const read = (relativePath) => readFileSync(resolve(root, relativePath), 'utf8');

test('后端发布模板通过摘要、SBOM、Provenance 和 canonical digest 门禁', () => {
  assert.deepEqual(checkBackendRelease({
    dockerfileSource: read('backend/Dockerfile'),
    workflowSource: read('.github/workflows/backend-quality.yml'),
  }), []);
});

test('后端发布门禁拒绝未固定摘要的基础镜像', () => {
  const failures = checkBackendRelease({
    dockerfileSource: 'FROM alpine:3.20\n',
    workflowSource: read('.github/workflows/backend-quality.yml'),
  });
  assert.ok(failures.some((failure) => failure.includes('sha256 digest')));
});

test('后端发布门禁要求脚本变更能够触发工作流', () => {
  const failures = checkBackendRelease({
    dockerfileSource: read('backend/Dockerfile'),
    workflowSource: read('.github/workflows/backend-quality.yml').replaceAll('tools/check-backend-release.*', ''),
  });
  assert.ok(failures.some((failure) => failure.includes('tools/check-backend-release.*')));
});
