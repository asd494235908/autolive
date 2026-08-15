import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const defaultConfigPath = resolve(desktopRoot, 'src-tauri', 'tauri.conf.json');
const semVerPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

function isStrictSemVer(version) {
  const match = typeof version === 'string' && semVerPattern.exec(version);
  return Boolean(
    match
      && (!match[4]
        || match[4].split('.').every((identifier) => (
          !/^\d+$/.test(identifier) || /^(0|[1-9]\d*)$/.test(identifier)
        ))),
  );
}

export function readDesktopVersion(configPath = defaultConfigPath) {
  const config = JSON.parse(readFileSync(configPath, 'utf8'));
  if (!isStrictSemVer(config.version)) {
    throw new Error(`Tauri 版本号无效：${config.version}`);
  }
  return { version: config.version, release: `v${config.version}` };
}

function main() {
  const [argument, ...rest] = process.argv.slice(2);
  if (rest.length > 0 || (argument && argument !== '--version' && argument !== '--release')) {
    throw new Error('只支持 --version 或 --release');
  }
  const version = readDesktopVersion();
  if (argument === '--version') {
    console.log(version.version);
  } else if (argument === '--release') {
    console.log(version.release);
  } else {
    console.log(JSON.stringify(version));
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
