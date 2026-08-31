const SHA256 = /^[0-9a-f]{64}$/;
const GIT_COMMIT = /^[0-9a-f]{40}$/;
const REQUIRED_COMPONENTS = ['ffmpeg', 'libplacebo', 'mpv'];
const RUNTIME_FILES = [
  'legal/Copyright.txt',
  'legal/GPL-2.0.txt',
  'legal/LGPL-2.1.txt',
  'legal/SOURCE.md',
  'legal/THIRD-PARTY-NOTICES.md',
  'mpv.exe',
  'spirv-cross-c-shared.dll',
  'vulkan-1.dll',
];
const SUPPLY_EVIDENCE = [
  'corresponding_source',
  'copyright_inventory',
  'cyclonedx_sbom',
  'license_inventory',
  'report',
  'spdx_sbom',
];

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function object(value, label) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value), `${label} 必须是对象`);
  return value;
}

function exactKeys(value, expected, label) {
  object(value, label);
  const actual = Object.keys(value).sort();
  const keys = [...expected].sort();
  assert(JSON.stringify(actual) === JSON.stringify(keys),
    `${label} 字段集合无效；拒绝缺失或未知字段`);
}

function sha256(value, label) {
  assert(typeof value === 'string' && SHA256.test(value), `${label} 必须是 64 位小写 SHA-256`);
}

function positiveInteger(value, label) {
  assert(Number.isSafeInteger(value) && value > 0, `${label}.size_bytes 必须是正安全整数`);
}

function descriptor(value, label, { withPath = false } = {}) {
  exactKeys(value, withPath ? ['path', 'size_bytes', 'sha256'] : ['size_bytes', 'sha256'], label);
  if (withPath) {
    assert(typeof value.path === 'string'
      && value.path.length > 0
      && value.path.length <= 256
      && /^[A-Za-z0-9._/-]+$/.test(value.path)
      && !value.path.includes('\\')
      && !value.path.startsWith('/')
      && !/^[A-Za-z]:/.test(value.path)
      && !value.path.split('/').some((part) => part === '' || part === '.' || part === '..'),
    `${label}.path 必须是规范的相对 POSIX 路径`);
  }
  positiveInteger(value.size_bytes, label);
  sha256(value.sha256, `${label}.sha256`);
}

function same(actual, expected, label) {
  assert(actual === expected, `${label} 与 expectedIdentity 不一致`);
}

function component(value, expected, label) {
  exactKeys(value, ['source_ref', 'license_expression'], label);
  exactKeys(expected, ['source_ref', 'license_expression'], `expectedIdentity.${label}`);
  assert(GIT_COMMIT.test(value.source_ref), `${label}.source_ref 必须是完整 40 位小写 Git 提交`);
  assert(GIT_COMMIT.test(expected.source_ref),
    `expectedIdentity.${label}.source_ref 必须是完整 40 位小写 Git 提交`);
  assert(typeof value.license_expression === 'string' && value.license_expression.length > 0,
    `${label}.license_expression 不能为空`);
  assert(typeof expected.license_expression === 'string' && expected.license_expression.length > 0,
    `expectedIdentity.${label}.license_expression 不能为空`);
  same(value.source_ref, expected.source_ref, `${label}.source_ref`);
  same(value.license_expression, expected.license_expression, `${label}.license_expression`);
}

function validateExpectedIdentity(expectedIdentity) {
  exactKeys(expectedIdentity, ['target', 'build', 'components', 'files', 'supply_evidence'],
    'expectedIdentity');
  assert(expectedIdentity.target === 'x86_64-pc-windows-msvc',
    'expectedIdentity.target 必须是 x86_64-pc-windows-msvc');
  exactKeys(expectedIdentity.build, ['lock_sha256', 'report_sha256'], 'expectedIdentity.build');
  sha256(expectedIdentity.build.lock_sha256, 'expectedIdentity.build.lock_sha256');
  sha256(expectedIdentity.build.report_sha256, 'expectedIdentity.build.report_sha256');

  object(expectedIdentity.components, 'expectedIdentity.components');
  for (const name of REQUIRED_COMPONENTS) {
    assert(Object.hasOwn(expectedIdentity.components, name),
      `expectedIdentity.components 缺少必需组件 ${name}`);
  }
  for (const [name, value] of Object.entries(expectedIdentity.components)) {
    component(value, value, `components.${name}`);
  }

  exactKeys(expectedIdentity.files, RUNTIME_FILES, 'expectedIdentity.files');
  for (const path of RUNTIME_FILES) descriptor(expectedIdentity.files[path], `expectedIdentity.files.${path}`);

  exactKeys(expectedIdentity.supply_evidence, SUPPLY_EVIDENCE, 'expectedIdentity.supply_evidence');
  for (const role of SUPPLY_EVIDENCE) {
    descriptor(expectedIdentity.supply_evidence[role],
      `expectedIdentity.supply_evidence.${role}`, { withPath: true });
  }
  same(expectedIdentity.supply_evidence.report.sha256,
    expectedIdentity.build.report_sha256, 'expectedIdentity.supply_evidence.report.sha256');
}

function validateAudit(audit, mode) {
  exactKeys(audit, [
    'release_review_status',
    'corresponding_source_complete',
    'third_party_notices_reviewed',
  ], 'audit');
  const blocked = audit.release_review_status === 'blocked'
    && audit.corresponding_source_complete === false
    && audit.third_party_notices_reviewed === false;
  const approved = audit.release_review_status === 'approved'
    && audit.corresponding_source_complete === true
    && audit.third_party_notices_reviewed === true;
  if (mode === 'release') assert(approved, 'release 模式要求 audit 为 approved/true/true');
  else assert(blocked || approved, 'technical 模式只允许完整 blocked/false/false 或 approved/true/true audit');
}

export function validateMpvRuntimeManifestV2(manifest, options) {
  exactKeys(options, ['mode', 'expectedIdentity'], 'options');
  const { mode, expectedIdentity } = options;
  assert(mode === 'technical' || mode === 'release', "mode 必须是 'technical' 或 'release'");
  validateExpectedIdentity(expectedIdentity);
  exactKeys(manifest, [
    'schema_version', 'target', 'build', 'components', 'audit', 'files', 'supply_evidence',
  ], 'manifest');
  assert(manifest.schema_version === 2, 'schema_version 必须严格等于 2');
  same(manifest.target, expectedIdentity.target, 'target');

  exactKeys(manifest.build, ['scope', 'claim', 'lock_sha256', 'report_sha256'], 'build');
  assert(manifest.build.scope === 'phase7a_supply_candidate',
    'build.scope 必须是 phase7a_supply_candidate');
  assert(manifest.build.claim === 'one_locked_cold_build_candidate',
    'build.claim 必须是 one_locked_cold_build_candidate');
  sha256(manifest.build.lock_sha256, 'build.lock_sha256');
  sha256(manifest.build.report_sha256, 'build.report_sha256');
  same(manifest.build.lock_sha256, expectedIdentity.build.lock_sha256, 'build.lock_sha256');
  same(manifest.build.report_sha256, expectedIdentity.build.report_sha256, 'build.report_sha256');

  exactKeys(manifest.components, Object.keys(expectedIdentity.components), 'components');
  for (const [name, expected] of Object.entries(expectedIdentity.components)) {
    component(manifest.components[name], expected, `components.${name}`);
  }

  exactKeys(manifest.files, RUNTIME_FILES, 'files');
  for (const path of RUNTIME_FILES) {
    descriptor(manifest.files[path], `files.${path}`);
    same(manifest.files[path].size_bytes, expectedIdentity.files[path].size_bytes,
      `files.${path}.size_bytes`);
    same(manifest.files[path].sha256, expectedIdentity.files[path].sha256,
      `files.${path}.sha256`);
  }

  exactKeys(manifest.supply_evidence, SUPPLY_EVIDENCE, 'supply_evidence');
  for (const role of SUPPLY_EVIDENCE) {
    descriptor(manifest.supply_evidence[role], `supply_evidence.${role}`, { withPath: true });
    for (const field of ['path', 'size_bytes', 'sha256']) {
      same(manifest.supply_evidence[role][field], expectedIdentity.supply_evidence[role][field],
        `supply_evidence.${role}.${field}`);
    }
  }
  same(manifest.supply_evidence.report.sha256, manifest.build.report_sha256,
    'supply_evidence.report.sha256');
  validateAudit(manifest.audit, mode);
  return manifest;
}
