import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const PRODUCTION = 'production';
const LOOPBACK_BINDINGS = new Set(['127.0.0.1', '::1', '[::1]']);

export function parseEnvFile(source) {
  const values = {};
  for (const originalLine of source.split(/\r?\n/u)) {
    const line = originalLine.trim();
    if (!line || line.startsWith('#')) continue;
    const match = line.match(/^(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)=(.*)$/u);
    if (!match) continue;
    let value = match[2].trim();
    if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
      value = value.slice(1, -1);
    }
    values[match[1]] = value;
  }
  return values;
}

function hasHttpsPublicURL(raw) {
  if (!raw) return false;
  try {
    const url = new URL(raw);
    return url.protocol === 'https:' && url.hostname !== '' && !url.username && !url.password && !url.search && !url.hash;
  } catch {
    return false;
  }
}

function parseHttpsPublicURL(raw) {
  if (!hasHttpsPublicURL(raw)) return null;
  return new URL(raw);
}

export function checkHttpsPublish({ env, reverseProxySource, capabilitySource, adminWebSource = '' }) {
  const failures = [];
  const publicURL = parseHttpsPublicURL(env.APP_PUBLIC_BASE_URL);
  if (env.APP_DEPLOYMENT_ENV !== PRODUCTION) {
    failures.push('APP_DEPLOYMENT_ENV must be production');
  }
  if (!hasHttpsPublicURL(env.APP_PUBLIC_BASE_URL)) {
    failures.push('APP_PUBLIC_BASE_URL must be an absolute https URL without credentials, query or fragment');
  }
  if (String(env.APP_ALLOW_INSECURE_HTTP ?? '').toLowerCase() !== 'false') {
    failures.push('APP_ALLOW_INSECURE_HTTP must be false');
  }
  if (!LOOPBACK_BINDINGS.has(env.AUTOLIVE_ADMIN_HTTP_BIND ?? '')) {
    failures.push('AUTOLIVE_ADMIN_HTTP_BIND must be a loopback address');
  }
  if (!/listen\s+443\s+ssl\s*;/u.test(reverseProxySource)) {
    failures.push('reverse proxy must listen on 443 with ssl');
  }
  if (!/return\s+308\s+https:\/\//u.test(reverseProxySource)) {
    failures.push('reverse proxy HTTP listener must redirect to HTTPS');
  }
  if (!/proxy_set_header\s+X-Forwarded-Proto\s+https\s*;/u.test(reverseProxySource)) {
    failures.push('reverse proxy must mark the upstream request as HTTPS');
  }
  if (adminWebSource && !/proxy_set_header\s+X-Forwarded-Proto\s+\$autolive_forwarded_proto\s*;/u.test(adminWebSource)) {
    failures.push('admin web proxy must preserve the trusted HTTPS forwarding marker');
  }
  if (publicURL && !new RegExp(`server_name\\s+[^;]*\\b${publicURL.hostname.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&')}\\b`, 'u').test(reverseProxySource)) {
    failures.push('reverse proxy server_name must include APP_PUBLIC_BASE_URL hostname');
  }

  let capability;
  try {
    capability = JSON.parse(capabilitySource);
  } catch {
    failures.push('Tauri capability is not valid JSON');
  }
  const urls = capability?.permissions
    ?.filter((permission) => typeof permission === 'object' && permission !== null)
    .flatMap((permission) => Array.isArray(permission.allow) ? permission.allow : [])
    .map((entry) => typeof entry === 'object' && entry !== null ? entry.url : undefined)
    .filter((url) => typeof url === 'string') ?? [];
  if (!urls.some((url) => /^https:\/\/[^/*]+(?::\d+)?\/\*\*$/u.test(url))) {
    failures.push('Tauri capability must allow one explicit HTTPS origin pattern');
  }
  if (publicURL && !urls.includes(`https://${publicURL.host}/**`)) {
    failures.push('Tauri capability HTTPS origin must match APP_PUBLIC_BASE_URL host');
  }
  if (urls.some((url) => url === 'http://*/**' || url === 'https://*/**')) {
    failures.push('Tauri capability must not allow wildcard HTTP origins');
  }
  return failures;
}

function valueAfterFlag(args, flag) {
  const index = args.indexOf(flag);
  if (index === -1 || !args[index + 1]) throw new Error(`${flag} is required`);
  return args[index + 1];
}

export function main(args = process.argv.slice(2)) {
  const envFile = resolve(valueAfterFlag(args, '--env-file'));
  const reverseProxyFile = resolve(valueAfterFlag(args, '--reverse-proxy'));
  const capabilityFile = resolve(valueAfterFlag(args, '--capability'));
  const adminWebFile = resolve(valueAfterFlag(args, '--admin-nginx'));
  const failures = checkHttpsPublish({
    env: parseEnvFile(readFileSync(envFile, 'utf8')),
    reverseProxySource: readFileSync(reverseProxyFile, 'utf8'),
    capabilitySource: readFileSync(capabilityFile, 'utf8'),
    adminWebSource: readFileSync(adminWebFile, 'utf8'),
  });
  if (failures.length > 0) {
    throw new Error(`HTTPS 发布门禁失败：\n- ${failures.join('\n- ')}`);
  }
  console.log('HTTPS 发布门禁通过：生产公共地址、反向代理 TLS、回环 HTTP hop 和桌面 capability 均已显式配置。');
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
