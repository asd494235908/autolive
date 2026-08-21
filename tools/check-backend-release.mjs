import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const DIGEST_IMAGE = /^(?:[\w./-]+)@sha256:[0-9a-f]{64}$/u;

export function checkBackendRelease({ dockerfileSource, workflowSource }) {
  const failures = [];
  const fromRefs = [...dockerfileSource.matchAll(/^FROM\s+(?:--platform=\$?\{?\w+\}?\s+)?([^\s]+)(?:\s+AS\s+[^\s]+)?$/gmu)].map((match) => match[1]);
  if (fromRefs.length < 2 || fromRefs.some((ref) => !DIGEST_IMAGE.test(ref))) {
    failures.push('every backend Dockerfile FROM image must use a sha256 digest');
  }
  for (const required of [
    'type=docker,dest=/tmp/autolive-api.docker.tar',
    'type=oci,dest=/tmp/autolive-api.oci.tar',
    'sbom: true',
    'provenance: mode=max',
    'buildx_digest=',
    'image_id=',
    'actions/upload-artifact@v4',
    'tools/check-backend-release.*',
  ]) {
    if (!workflowSource.includes(required)) {
      failures.push(`backend quality workflow is missing ${required}`);
    }
  }
  return failures;
}

function valueAfterFlag(args, flag) {
  const index = args.indexOf(flag);
  if (index === -1 || !args[index + 1]) throw new Error(`${flag} is required`);
  return args[index + 1];
}

export function main(args = process.argv.slice(2)) {
  const dockerfilePath = resolve(valueAfterFlag(args, '--dockerfile'));
  const workflowPath = resolve(valueAfterFlag(args, '--workflow'));
  const failures = checkBackendRelease({
    dockerfileSource: readFileSync(dockerfilePath, 'utf8'),
    workflowSource: readFileSync(workflowPath, 'utf8'),
  });
  if (failures.length > 0) {
    throw new Error(`后端发布可复现性门禁失败：\n- ${failures.join('\n- ')}`);
  }
  console.log('后端发布可复现性门禁通过：基础镜像摘要、OCI/Docker 产物、SBOM、Provenance 和内容摘要均已声明。');
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
