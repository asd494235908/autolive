import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { checkHttpsPublish, parseEnvFile } from './check-https-publish.mjs';

const root = resolve(fileURLToPath(new URL('..', import.meta.url)));

function read(relativePath) {
  return readFileSync(resolve(root, relativePath), 'utf8');
}

test('仓库生产模板通过 HTTPS 发布门禁', () => {
  const failures = checkHttpsPublish({
    env: parseEnvFile(read('deploy/.env.example')),
    reverseProxySource: read('deploy/reverse-proxy/nginx.conf'),
    capabilitySource: read('desktop/src-tauri/capabilities/default.json'),
    adminWebSource: read('admin-web/nginx.conf'),
  });
  assert.deepEqual(failures, []);
});

test('发布门禁拒绝公网 HTTP 与非回环管理端绑定', () => {
  const failures = checkHttpsPublish({
    env: {
      APP_DEPLOYMENT_ENV: 'production',
      APP_PUBLIC_BASE_URL: 'http://admin.example.com',
      APP_ALLOW_INSECURE_HTTP: 'true',
      AUTOLIVE_ADMIN_HTTP_BIND: '0.0.0.0',
    },
    reverseProxySource: read('deploy/reverse-proxy/nginx.conf'),
    capabilitySource: read('desktop/src-tauri/capabilities/default.json'),
    adminWebSource: read('admin-web/nginx.conf'),
  });
  assert.ok(failures.some((failure) => failure.includes('APP_PUBLIC_BASE_URL')));
  assert.ok(failures.some((failure) => failure.includes('APP_ALLOW_INSECURE_HTTP')));
  assert.ok(failures.some((failure) => failure.includes('AUTOLIVE_ADMIN_HTTP_BIND')));
});

test('发布门禁要求代理和桌面 HTTPS origin 与公共域名一致', () => {
  const failures = checkHttpsPublish({
    env: {
      APP_DEPLOYMENT_ENV: 'production',
      APP_PUBLIC_BASE_URL: 'https://other.example.com',
      APP_ALLOW_INSECURE_HTTP: 'false',
      AUTOLIVE_ADMIN_HTTP_BIND: '127.0.0.1',
    },
    reverseProxySource: read('deploy/reverse-proxy/nginx.conf'),
    capabilitySource: read('desktop/src-tauri/capabilities/default.json'),
    adminWebSource: read('admin-web/nginx.conf'),
  });
  assert.ok(failures.some((failure) => failure.includes('server_name')));
  assert.ok(failures.some((failure) => failure.includes('Tauri capability HTTPS origin')));
});

test('env parser ignores comments and removes matching quotes', () => {
  assert.deepEqual(parseEnvFile('# comment\nexport A="value"\nB=plain # trailing text'), {
    A: 'value',
    B: 'plain # trailing text',
  });
});
