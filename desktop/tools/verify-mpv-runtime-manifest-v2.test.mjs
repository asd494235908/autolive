import assert from 'node:assert/strict';
import test from 'node:test';

import { validateMpvRuntimeManifestV2 } from './verify-mpv-runtime-manifest-v2.mjs';

const hashes = {
  lock: '1'.repeat(64),
  report: '2'.repeat(64),
  mpv: '3'.repeat(64),
  spirv: '4'.repeat(64),
  vulkan: '5'.repeat(64),
  copyright: '6'.repeat(64),
  gpl: '7'.repeat(64),
  lgpl: '8'.repeat(64),
  source: '9'.repeat(64),
  notices: 'a'.repeat(64),
  correspondingSource: 'b'.repeat(64),
  cyclonedx: 'c'.repeat(64),
  spdx: 'd'.repeat(64),
  licenses: 'e'.repeat(64),
  copyrights: 'f'.repeat(64),
};

const components = {
  mpv: {
    source_ref: '7b8915bc1d04c7e1b61184e00c7fbfaab1911e75',
    license_expression: 'GPL-2.0-or-later',
  },
  libplacebo: {
    source_ref: '22ee762e8e0890fc54068beb670310f0edce7263',
    license_expression: 'LGPL-2.1-or-later',
  },
  ffmpeg: {
    source_ref: '1d7b14f61d66fdf18f15204c613df9d65396c319',
    license_expression: 'LGPL-2.1-or-later',
  },
};

const files = {
  'mpv.exe': { size_bytes: 37_645_824, sha256: hashes.mpv },
  'spirv-cross-c-shared.dll': { size_bytes: 1_960_960, sha256: hashes.spirv },
  'vulkan-1.dll': { size_bytes: 663_552, sha256: hashes.vulkan },
  'legal/Copyright.txt': { size_bytes: 1, sha256: hashes.copyright },
  'legal/GPL-2.0.txt': { size_bytes: 2, sha256: hashes.gpl },
  'legal/LGPL-2.1.txt': { size_bytes: 3, sha256: hashes.lgpl },
  'legal/SOURCE.md': { size_bytes: 4, sha256: hashes.source },
  'legal/THIRD-PARTY-NOTICES.md': { size_bytes: 5, sha256: hashes.notices },
};

const supplyEvidence = {
  report: {
    path: 'reproducible-build-report.json', size_bytes: 10, sha256: hashes.report,
  },
  corresponding_source: {
    path: 'build-evidence/corresponding-source.tar.zst', size_bytes: 11,
    sha256: hashes.correspondingSource,
  },
  cyclonedx_sbom: {
    path: 'build-evidence/sbom.cdx.json', size_bytes: 12, sha256: hashes.cyclonedx,
  },
  spdx_sbom: {
    path: 'build-evidence/sbom.spdx.json', size_bytes: 13, sha256: hashes.spdx,
  },
  license_inventory: {
    path: 'build-evidence/license-inventory.txt', size_bytes: 14, sha256: hashes.licenses,
  },
  copyright_inventory: {
    path: 'build-evidence/copyright-inventory.txt', size_bytes: 15, sha256: hashes.copyrights,
  },
};

function fixture() {
  const expectedIdentity = structuredClone({
    target: 'x86_64-pc-windows-msvc',
    build: { lock_sha256: hashes.lock, report_sha256: hashes.report },
    components,
    files,
    supply_evidence: supplyEvidence,
  });
  const manifest = structuredClone({
    schema_version: 2,
    target: expectedIdentity.target,
    build: {
      scope: 'phase7a_supply_candidate',
      claim: 'one_locked_cold_build_candidate',
      ...expectedIdentity.build,
    },
    components: expectedIdentity.components,
    audit: {
      release_review_status: 'blocked',
      corresponding_source_complete: false,
      third_party_notices_reviewed: false,
    },
    files: expectedIdentity.files,
    supply_evidence: expectedIdentity.supply_evidence,
  });
  return { manifest, expectedIdentity };
}

function rejects(mutator, pattern, options = {}) {
  const { manifest, expectedIdentity } = fixture();
  mutator(manifest, expectedIdentity);
  assert.throws(
    () => validateMpvRuntimeManifestV2(manifest, {
      mode: options.mode ?? 'technical', expectedIdentity,
    }),
    pattern,
  );
}

test('technical 模式接受完整 blocked 候选', () => {
  const { manifest, expectedIdentity } = fixture();
  assert.equal(
    validateMpvRuntimeManifestV2(manifest, { mode: 'technical', expectedIdentity }),
    manifest,
  );
});

test('release 模式只接受完整 approved 审计状态', () => {
  const { manifest, expectedIdentity } = fixture();
  manifest.audit = {
    release_review_status: 'approved',
    corresponding_source_complete: true,
    third_party_notices_reviewed: true,
  };
  assert.equal(
    validateMpvRuntimeManifestV2(manifest, { mode: 'release', expectedIdentity }),
    manifest,
  );
  assert.equal(
    validateMpvRuntimeManifestV2(manifest, { mode: 'technical', expectedIdentity }),
    manifest,
  );
});

test('拒绝 release blocked 与半批准审计状态', () => {
  rejects(() => {}, /release.*approved/i, { mode: 'release' });
  rejects((manifest) => {
    manifest.audit.corresponding_source_complete = true;
  }, /audit/i);
});

test('拒绝未知字段，包括嵌套未知字段', () => {
  rejects((manifest) => { manifest.artifact = {}; }, /未知字段|字段集合/);
  rejects((manifest) => { manifest.build.release = 'legacy'; }, /未知字段|字段集合/);
  rejects((manifest) => { manifest.components.mpv.version = 'legacy'; }, /未知字段|字段集合/);
  rejects((manifest) => { manifest.files['mpv.exe'].mode = 'executable'; }, /未知字段|字段集合/);
});

test('拒绝旧 schema、错误 target、scope 和 claim', () => {
  rejects((manifest) => { manifest.schema_version = 1; }, /schema_version/);
  rejects((manifest) => { manifest.target = 'legacy'; }, /target/);
  rejects((manifest) => { manifest.build.scope = 'legacy'; }, /scope/);
  rejects((manifest) => { manifest.build.claim = 'legacy'; }, /claim/);
});

test('拒绝调用方用其他目标自证身份和证据路径控制字符', () => {
  rejects((manifest, expectedIdentity) => {
    manifest.target = 'aarch64-apple-darwin';
    expectedIdentity.target = 'aarch64-apple-darwin';
  }, /x86_64-pc-windows-msvc/);
  rejects((manifest, expectedIdentity) => {
    const path = 'build-evidence/report\u0000.json';
    manifest.supply_evidence.report.path = path;
    expectedIdentity.supply_evidence.report.path = path;
  }, /path/);
});

test('拒绝短提交及缺少必需组件', () => {
  rejects((manifest) => { manifest.components.mpv.source_ref = '7b8915bc1d'; }, /source_ref/);
  rejects((manifest, expectedIdentity) => {
    delete manifest.components.ffmpeg;
    delete expectedIdentity.components.ffmpeg;
  }, /ffmpeg/);
});

test('组件可由 expectedIdentity 扩展，但 manifest 不能擅自增减', () => {
  const extra = {
    source_ref: '0'.repeat(40),
    license_expression: 'Apache-2.0',
  };
  const { manifest, expectedIdentity } = fixture();
  manifest.components.shaderc = extra;
  expectedIdentity.components.shaderc = structuredClone(extra);
  assert.equal(
    validateMpvRuntimeManifestV2(manifest, { mode: 'technical', expectedIdentity }),
    manifest,
  );
  rejects((candidate) => { candidate.components.shaderc = extra; }, /components/);
});

test('拒绝缺少、多余或描述无效的运行文件', () => {
  rejects((manifest) => { delete manifest.files['vulkan-1.dll']; }, /files/);
  rejects((manifest) => {
    manifest.files['d3dcompiler_43.dll'] = { size_bytes: 1, sha256: '0'.repeat(64) };
  }, /files/);
  rejects((manifest) => { manifest.files['mpv.exe'].size_bytes = 0; }, /size_bytes/);
});

test('拒绝缺少、多余、路径越界或描述无效的供应证据', () => {
  rejects((manifest) => { delete manifest.supply_evidence.spdx_sbom; }, /supply_evidence/);
  rejects((manifest) => {
    manifest.supply_evidence.build_log = {
      path: 'build-log.txt', size_bytes: 1, sha256: '0'.repeat(64),
    };
  }, /supply_evidence/);
  rejects((manifest) => {
    manifest.supply_evidence.report.path = '../reproducible-build-report.json';
  }, /path/);
  rejects((manifest) => { manifest.supply_evidence.report.size_bytes = -1; }, /size_bytes/);
});

test('拒绝被篡改的 build、运行文件和供应证据哈希', () => {
  rejects((manifest) => { manifest.build.lock_sha256 = '0'.repeat(64); }, /lock_sha256/);
  rejects((manifest) => { manifest.files['mpv.exe'].sha256 = '0'.repeat(64); }, /mpv\.exe/);
  rejects((manifest) => {
    manifest.supply_evidence.cyclonedx_sbom.sha256 = '0'.repeat(64);
  }, /cyclonedx_sbom/);
});

test('拒绝缺失或不可信的 expectedIdentity 及非法 mode', () => {
  const { manifest, expectedIdentity } = fixture();
  assert.throws(
    () => validateMpvRuntimeManifestV2(manifest, { mode: 'technical' }),
    /options|expectedIdentity/,
  );
  expectedIdentity.components.mpv.license_expression = '';
  assert.throws(
    () => validateMpvRuntimeManifestV2(manifest, { mode: 'technical', expectedIdentity }),
    /license_expression/,
  );
  assert.throws(
    () => validateMpvRuntimeManifestV2(manifest, { mode: 'preview', expectedIdentity }),
    /mode/,
  );
});
