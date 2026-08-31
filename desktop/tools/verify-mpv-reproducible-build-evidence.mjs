import { createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import { copyFile, lstat, mkdir, mkdtemp, open, readFile, readdir, realpath, rm, stat } from 'node:fs/promises';
import { basename, dirname, isAbsolute, join, posix, relative, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { promisify } from 'node:util';

const exec = promisify(execFile);
const SHA256 = /^[a-f0-9]{64}$/;
const RECIPE_PREFIX = 'desktop/third_party/mpv/build/';
const PROJECTS = ['freetype', 'fribidi', 'harfbuzz', 'libass', 'lcms2', 'libplacebo', 'mpv'];
const INTROSPECTION_KINDS = ['buildoptions', 'dependencies', 'targets', 'projectinfo', 'machines'];
const RUNTIME_FILES = Object.freeze({
  'mpv-executable': 'mpv.exe',
  'spirv-cross-runtime': 'spirv-cross-c-shared.dll',
  'vulkan-loader-runtime': 'vulkan-1.dll',
});
const ARCHIVE_LIMITS = Object.freeze({
  maximumMembers: 500_000,
  maximumMemberBytes: 4 * 1024 * 1024 * 1024,
  maximumExpandedBytes: 12 * 1024 * 1024 * 1024,
});
const VENDORED_SOURCE_TREES = Object.freeze([
  ['shaderc', 'third_party/glslang', 'glslang'],
  ['shaderc', 'third_party/spirv-tools', 'spirv-tools'],
  ['shaderc', 'third_party/spirv-headers', 'spirv-headers'],
  ['libplacebo', '3rdparty/Vulkan-Headers', 'vulkan-headers'],
  ['libplacebo', '3rdparty/fast_float', 'fast-float'],
  ['libplacebo', '3rdparty/jinja', 'jinja'],
  ['libplacebo', '3rdparty/markupsafe', 'markupsafe'],
]);

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function hashBytes(value) {
  return createHash('sha256').update(value).digest('hex');
}

async function jsonFile(path, maximumBytes = 32 * 1024 * 1024) {
  const stat = await lstat(path);
  assert(stat.isFile() && !stat.isSymbolicLink() && stat.size <= maximumBytes, `${path} 不是有界普通 JSON 文件`);
  return JSON.parse(await readFile(path, 'utf8'));
}

function canonicalJson(value) {
  if (Array.isArray(value)) return value.map(canonicalJson);
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(
      Object.keys(value).sort().map((key) => [key, canonicalJson(value[key])]),
    );
  }
  return value;
}

export function semanticJsonEqual(actual, expected) {
  return JSON.stringify(canonicalJson(actual)) === JSON.stringify(canonicalJson(expected));
}

function exactJson(actual, expected, label) {
  assert(semanticJsonEqual(actual, expected), `${label} 与输入锁不一致`);
}

function sourceVersion(source) {
  return source.commit ?? source.sha256;
}

function sourceRef(source) {
  return `source:${source.name}@${sourceVersion(source)}`;
}

function toolRef(name, tool) {
  return `tool:${name}@${tool.version}`;
}

function spdxId(ref) {
  return `SPDXRef-${ref.replaceAll(/[^A-Za-z0-9.-]/g, '-')}`;
}

function propertiesMap(component) {
  return new Map((component.properties ?? []).map((property) => [property.name, property.value]));
}

function parsePe(buffer, label) {
  assert(buffer.length >= 512 && buffer.toString('ascii', 0, 2) === 'MZ', `${label} 缺少 MZ 头`);
  const peOffset = buffer.readUInt32LE(0x3c);
  assert(peOffset + 24 < buffer.length && buffer.toString('ascii', peOffset, peOffset + 4) === 'PE\0\0', `${label} 缺少 PE 头`);
  const machine = buffer.readUInt16LE(peOffset + 4);
  const sectionCount = buffer.readUInt16LE(peOffset + 6);
  const optionalSize = buffer.readUInt16LE(peOffset + 20);
  const optional = peOffset + 24;
  assert(machine === 0x8664 && buffer.readUInt16LE(optional) === 0x20b, `${label} 必须是 x64 PE32+`);
  const subsystem = buffer.readUInt16LE(optional + 68);
  const importRva = buffer.readUInt32LE(optional + 112 + 8);
  const sections = [];
  const sectionTable = optional + optionalSize;
  for (let index = 0; index < sectionCount; index += 1) {
    const offset = sectionTable + index * 40;
    assert(offset + 40 <= buffer.length, `${label} 节表越界`);
    sections.push({
      virtualSize: buffer.readUInt32LE(offset + 8),
      virtualAddress: buffer.readUInt32LE(offset + 12),
      rawSize: buffer.readUInt32LE(offset + 16),
      rawOffset: buffer.readUInt32LE(offset + 20),
    });
  }
  function rvaOffset(rva) {
    const section = sections.find((item) => rva >= item.virtualAddress
      && rva < item.virtualAddress + Math.max(item.virtualSize, item.rawSize));
    assert(section, `${label} RVA 不在任何节内`);
    const offset = section.rawOffset + rva - section.virtualAddress;
    assert(offset >= 0 && offset < buffer.length, `${label} RVA 越界`);
    return offset;
  }
  const imports = [];
  if (importRva !== 0) {
    let descriptor = rvaOffset(importRva);
    for (let count = 0; count < 1024; count += 1, descriptor += 20) {
      assert(descriptor + 20 <= buffer.length, `${label} 导入表越界`);
      const fields = [0, 4, 8, 12, 16].map((delta) => buffer.readUInt32LE(descriptor + delta));
      if (fields.every((value) => value === 0)) break;
      let cursor = rvaOffset(fields[3]);
      let end = cursor;
      while (end < buffer.length && end - cursor < 260 && buffer[end] !== 0) end += 1;
      assert(end < buffer.length && buffer[end] === 0, `${label} DLL 名无终止符`);
      imports.push(buffer.toString('ascii', cursor, end).toUpperCase());
    }
  }
  return { machine, subsystem, imports: [...new Set(imports)].sort() };
}

function safeArchivePath(value, label) {
  assert(value && !value.includes('\\') && !value.startsWith('/') && !/^[A-Za-z]:/.test(value), `${label} 包含绝对路径`);
  const normalized = posix.normalize(value);
  assert(normalized !== '..' && !normalized.startsWith('../') && !isAbsolute(normalized), `${label} 包含路径穿越`);
  return normalized.replace(/^\.\//, '');
}

export function parseArchiveManifest(stdout, label, limits = ARCHIVE_LIMITS) {
  const lines = stdout.split(/\r?\n/).filter(Boolean);
  assert(lines.length > 0 && lines.length <= limits.maximumMembers, `${label} 成员数量超过上限`);
  const entries = [];
  let expandedBytes = 0;
  for (const line of lines) {
    const match = /^(.)(?:\S*)\s+\d+\s+\S+\s+\S+\s+(\d+)\s+\S+\s+\S+\s+\S+\s(.+)$/.exec(line);
    assert(match, `${label} 无法解析受控归档清单：${line}`);
    const [, marker, sizeText, tail] = match;
    const size = Number(sizeText);
    assert(Number.isSafeInteger(size) && size >= 0 && size <= limits.maximumMemberBytes, `${label} 单成员展开大小超过上限`);
    let type;
    let pathText = tail;
    let target = null;
    if (marker === 'l') {
      type = 'symlink';
      const separator = tail.lastIndexOf(' -> ');
      assert(separator > 0, `${label} symlink 清单无目标`);
      pathText = tail.slice(0, separator);
      target = tail.slice(separator + 4);
    } else if (tail.includes(' link to ')) {
      type = 'hardlink';
      const separator = tail.lastIndexOf(' link to ');
      pathText = tail.slice(0, separator);
      target = tail.slice(separator + 9);
    } else if (marker === '-') type = 'file';
    else if (marker === 'd') type = 'directory';
    else throw new Error(`${label} 禁止特殊成员类型：${marker}`);
    const path = safeArchivePath(pathText, `${label} member`);
    if (target !== null) {
      assert(target && !target.includes('\\') && !target.startsWith('/') && !/^[A-Za-z]:/.test(target), `${label} 链接目标必须是相对路径`);
      const resolved = type === 'symlink' ? posix.normalize(posix.join(posix.dirname(path), target)) : posix.normalize(target);
      assert(resolved !== '..' && !resolved.startsWith('../') && !isAbsolute(resolved), `${label} 链接目标越过归档根：${path} -> ${target}`);
    }
    if (type === 'file') {
      expandedBytes += size;
      assert(expandedBytes <= limits.maximumExpandedBytes, `${label} 总展开字节超过上限`);
    }
    entries.push({ type, path, target, size });
  }
  return entries;
}

function stripArchivePath(path, count) {
  return path.split('/').slice(count).join('/');
}

export async function materializeArchiveLinks({ root, manifest, stripComponents = 0 }) {
  for (const entry of manifest.filter((item) => item.type === 'symlink' || item.type === 'hardlink')) {
    const resolvedTarget = entry.type === 'symlink'
      ? posix.normalize(posix.join(posix.dirname(entry.path), entry.target))
      : posix.normalize(entry.target);
    const destination = stripArchivePath(entry.path, stripComponents);
    const target = stripArchivePath(resolvedTarget, stripComponents);
    assert(destination && target, `归档链接 strip-components 无效：${entry.path}`);
    const destinationPath = join(root, ...destination.split('/'));
    const targetPath = join(root, ...target.split('/'));
    await mkdir(dirname(destinationPath), { recursive: true });
    await copyFile(targetPath, destinationPath);
  }
}

export async function extractArchive(archive, label, {
  stripComponents = 0,
  limits = ARCHIVE_LIMITS,
  temporaryRoot = tmpdir(),
  execute = exec,
} = {}) {
  const { stdout } = await execute('tar', ['-tvf', archive], { maxBuffer: 128 * 1024 * 1024, windowsHide: true });
  const manifest = parseArchiveManifest(stdout, label, limits);
  const root = await mkdtemp(join(temporaryRoot, 'archive-'));
  const args = ['-xf', archive, '-C', root];
  if (stripComponents > 0) args.push(`--strip-components=${stripComponents}`);
  const materializedLinks = process.platform === 'win32'
    ? manifest.filter((item) => item.type === 'symlink' || item.type === 'hardlink')
    : [];
  for (const entry of materializedLinks) args.push(`--exclude=${entry.path}`);
  try {
    await execute('tar', args, { maxBuffer: 16 * 1024 * 1024, windowsHide: true });
    if (materializedLinks.length > 0) {
      await materializeArchiveLinks({ root, manifest: materializedLinks, stripComponents });
    }
    return { root, manifest };
  } catch (error) {
    await cleanupTemporary(root, temporaryRoot, 'archive-');
    throw error;
  }
}

async function regularFiles(root) {
  const files = [];
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isSymbolicLink()) {
        const canonical = await realpath(path);
        const child = relative(await realpath(root), canonical);
        assert(child !== '..' && !child.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`) && !isAbsolute(child), `归档链接解包后越界：${path}`);
      } else if (entry.isDirectory()) await visit(path);
      else {
        assert(entry.isFile(), `归档只允许普通文件/目录：${path}`);
        files.push(relative(root, path).replaceAll('\\', '/'));
      }
    }
  }
  await visit(root);
  return files.sort();
}

async function cleanupTemporary(root, temporaryRoot = tmpdir(), prefix = 'archive-') {
  const canonicalTemp = await realpath(temporaryRoot);
  const canonicalRoot = await realpath(root);
  const child = relative(canonicalTemp, canonicalRoot);
  assert(child.startsWith(prefix) && !child.includes('..') && !isAbsolute(child), `拒绝清理非 Phase 7A 临时目录：${canonicalRoot}`);
  await rm(canonicalRoot, { recursive: true, force: true });
}

export async function verifyPatchApplies({ sourceRoot, patchPath, temporaryRoot, execute = exec }) {
  const isolatedGit = {
    env: { ...process.env, GIT_CEILING_DIRECTORIES: temporaryRoot },
    maxBuffer: 8 * 1024 * 1024,
    windowsHide: true,
  };
  const git = ['-C', sourceRoot, '-c', 'core.autocrlf=false', '-c', 'core.eol=lf'];
  await execute('git', [...git, 'init', '--quiet'], isolatedGit);
  await execute('git', [...git, 'apply', '--check', '--unidiff-zero', patchPath], isolatedGit);
  await execute('git', [...git, 'apply', '--unidiff-zero', patchPath], isolatedGit);
}

async function fileDigest(path) {
  const hash = createHash('sha256');
  const handle = await open(path, 'r');
  try {
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let position = 0;
    while (true) {
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, position);
      if (bytesRead === 0) return hash.digest('hex');
      hash.update(buffer.subarray(0, bytesRead));
      position += bytesRead;
    }
  } finally {
    await handle.close();
  }
}

function excludedTreePath(path, excludedPrefixes) {
  return excludedPrefixes.some((prefix) => path === prefix || path.startsWith(`${prefix}/`));
}

export async function sourceTreeManifest(root, { excludedPrefixes = [] } = {}) {
  const canonicalRoot = await realpath(root);
  const rows = [];
  async function visit(directory, prefix = '') {
    const entries = await readdir(directory, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      const relativePath = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (excludedTreePath(relativePath, excludedPrefixes)) continue;
      const path = join(directory, entry.name);
      if (entry.isSymbolicLink()) {
        const canonical = await realpath(path);
        const child = relative(canonicalRoot, canonical);
        assert(child !== '..' && !child.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`) && !isAbsolute(child), `源码树链接越界：${path}`);
        const target = await stat(path);
        assert(target.isFile(), `源码树只允许文件 symlink：${path}`);
        rows.push({ path: relativePath, size: Number(target.size), sha256: await fileDigest(path) });
      } else if (entry.isDirectory()) {
        await visit(path, relativePath);
      } else {
        assert(entry.isFile(), `源码树只允许普通文件、目录或内部文件 symlink：${path}`);
        const metadata = await lstat(path);
        rows.push({ path: relativePath, size: Number(metadata.size), sha256: await fileDigest(path) });
      }
    }
  }
  await visit(root);
  return rows;
}

export async function verifySourceTreeContains({ actualRoot, expectedRoot, excludedPrefixes = [], label }) {
  const [actual, expected] = await Promise.all([
    sourceTreeManifest(actualRoot, { excludedPrefixes }),
    sourceTreeManifest(expectedRoot, { excludedPrefixes }),
  ]);
  const actualByPath = new Map(actual.map((item) => [item.path, item]));
  const consumedAliases = new Set();
  for (const expectedItem of expected) {
    let actualItem = actualByPath.get(expectedItem.path);
    if (!actualItem && !/^[\x20-\x7e]+$/.test(expectedItem.path)) {
      const candidates = actual.filter((item) => !consumedAliases.has(item.path)
        && posix.dirname(item.path) === posix.dirname(expectedItem.path)
        && item.size === expectedItem.size
        && item.sha256 === expectedItem.sha256);
      assert(candidates.length === 1, `${label} 非 ASCII 路径无法唯一对应：${expectedItem.path}`);
      [actualItem] = candidates;
      consumedAliases.add(actualItem.path);
    }
    assert(actualItem, `${label} 缺少锁定文件：${expectedItem.path}`);
    assert(
      actualItem.size === expectedItem.size && actualItem.sha256 === expectedItem.sha256,
      `${label} 锁定文件字节不一致：${expectedItem.path}`,
    );
  }
}

function parseTsv(value, header, label) {
  const lines = value.trimEnd().split(/\r?\n/);
  assert(lines.shift() === header.join('\t'), `${label} 表头无效`);
  return lines.map((line) => {
    const fields = line.split('\t');
    assert(fields.length === header.length, `${label} 行字段数量无效`);
    return Object.fromEntries(header.map((name, index) => [name, fields[index]]));
  });
}

async function verifyPatchArchives({ lock, lockBytes, evidenceRoot, buildInputsRoot, temporaryRoot }) {
  const patchArchive = await extractArchive(join(evidenceRoot, 'patch-bundle.tar.zst'), 'patch bundle', { temporaryRoot });
  let sourceArchive;
  try {
    sourceArchive = await extractArchive(join(evidenceRoot, 'corresponding-source.tar.zst'), 'corresponding source', { temporaryRoot });
    const patchFiles = await regularFiles(patchArchive.root);
    const expectedPatchFiles = lock.patches.map((patch) => `patches/${basename(patch.path)}`).sort();
    exactJson(patchFiles, expectedPatchFiles, 'patch bundle 文件集合');
    for (const patch of lock.patches) {
      const bundled = join(patchArchive.root, 'patches', basename(patch.path));
      const recipe = join(sourceArchive.root, 'recipe', patch.path.slice(RECIPE_PREFIX.length));
      assert(hashBytes(await readFile(bundled)) === patch.sha256, `patch bundle 哈希不匹配：${patch.path}`);
      assert(hashBytes(await readFile(recipe)) === patch.sha256, `对应源码配方补丁哈希不匹配：${patch.path}`);
    }
    const sourceFiles = await regularFiles(sourceArchive.root);
    const archivedRecipes = sourceFiles.filter((path) => path.startsWith('recipe/'))
      .map((path) => `${RECIPE_PREFIX}${path.slice('recipe/'.length)}`).sort();
    exactJson(archivedRecipes, lock.recipe_inventory.map((item) => item.path).sort(), '对应源码 recipe_inventory');
    const archivedLock = await readFile(join(sourceArchive.root, 'build-lock', 'reproducible-build-lock.json'));
    assert(archivedLock.equals(lockBytes), '对应源码中的输入锁与当前锁不一致');
    for (const source of lock.sources) {
      const original = await extractArchive(join(buildInputsRoot, ...source.cache_path.split('/')), `source ${source.name}`, { stripComponents: 1, temporaryRoot });
      try {
        for (const patch of lock.patches.filter((item) => item.component === source.name).sort((left, right) => left.order - right.order)) {
          await verifyPatchApplies({
            sourceRoot: original.root,
            patchPath: join(patchArchive.root, 'patches', basename(patch.path)),
            temporaryRoot,
          });
        }
        const excludedPrefixes = ['.git', ...VENDORED_SOURCE_TREES
          .filter(([owner]) => owner === source.name)
          .map(([, destination]) => destination)];
        await verifySourceTreeContains({
          actualRoot: join(sourceArchive.root, 'sources', source.name),
          expectedRoot: original.root,
          excludedPrefixes,
          label: `对应源码完整树：${source.name}`,
        });
      } finally {
        await cleanupTemporary(original.root, temporaryRoot);
      }
    }
    for (const [owner, destination, source] of VENDORED_SOURCE_TREES) {
      await verifySourceTreeContains({
        actualRoot: join(sourceArchive.root, 'sources', owner, ...destination.split('/')),
        expectedRoot: join(sourceArchive.root, 'sources', source),
        label: `对应源码 vendoring：${owner}/${destination}`,
      });
    }
    return sourceArchive.root;
  } catch (error) {
    if (sourceArchive) await cleanupTemporary(sourceArchive.root, temporaryRoot);
    throw error;
  } finally {
    await cleanupTemporary(patchArchive.root, temporaryRoot);
  }
}

async function verifyCopyright(lock, evidenceRoot, sourceRoot) {
  const rows = parseTsv(
    await readFile(join(evidenceRoot, 'copyright-inventory.txt'), 'utf8'),
    ['component', 'path', 'sha256'], 'copyright inventory',
  );
  const expected = [];
  for (const source of lock.sources) {
    const root = join(sourceRoot, 'sources', source.name);
    for (const path of await regularFiles(root)) {
      if (/^(?:license|copying|copyright)/i.test(basename(path))) {
        expected.push({ component: source.name, path, sha256: hashBytes(await readFile(join(root, ...path.split('/')))) });
      }
    }
  }
  exactJson(rows, expected, 'copyright inventory 与对应源码嵌套许可证');
}

async function verifyStructuredEvidence({ lock, lockBytes, evidenceRoot }) {
  const lockHash = hashBytes(lockBytes);
  const dependency = await jsonFile(join(evidenceRoot, 'dependency-lock.json'));
  assert(dependency.schemaVersion === 2 && dependency.target === lock.target && dependency.lockSha256 === lockHash, 'dependency lock 身份无效');
  exactJson(dependency.inputs, lock.cache_inventory.map((path) => {
    const source = lock.sources.find((item) => item.cache_path === path);
    const tool = Object.values(lock.toolchain.tools).find((item) => item.cache_path === path);
    const item = source ?? tool;
    return { path, sizeBytes: item.size_bytes, sha256: item.sha256 };
  }), 'dependency lock inputs');
  exactJson(dependency.patches, lock.patches, 'dependency lock patches');
  exactJson(dependency.recipeInventory, lock.recipe_inventory, 'dependency lock recipe inventory');

  const parameters = await jsonFile(join(evidenceRoot, 'build-parameters.json'));
  assert(parameters.schemaVersion === 2 && parameters.target === lock.target && parameters.network === 'none', 'build parameters 身份无效');
  exactJson(parameters.mesonArguments, lock.build_recipe.meson_arguments, 'Meson 参数');
  exactJson(parameters.spirvCrossCmakeArguments, lock.build_recipe.spirv_cross_cmake_arguments, 'SPIRV-Cross 参数');
  exactJson(parameters.ffmpegArguments, lock.build_recipe.ffmpeg_arguments, 'FFmpeg 参数');

  const meson = await jsonFile(join(evidenceRoot, 'meson-introspection.json'), 128 * 1024 * 1024);
  assert(meson.schemaVersion === 2, 'Meson introspection schema 无效');
  exactJson(Object.keys(meson.projects).sort(), [...PROJECTS].sort(), 'Meson 项目集合');
  for (const project of PROJECTS) {
    exactJson(Object.keys(meson.projects[project]).sort(), [...INTROSPECTION_KINDS].sort(), `Meson ${project} introspection 集合`);
  }
  const options = new Map(meson.projects.mpv.buildoptions.map((option) => [option.name, option.value]));
  for (const feature of lock.features.required) {
    const expected = feature === 'cplayer' ? true : 'enabled';
    assert(options.get(feature) === expected, `Meson 未启用锁定功能：${feature}`);
  }
  for (const feature of lock.features.disabled) {
    const expected = feature === 'libmpv' ? false : 'disabled';
    assert(options.get(feature) === expected, `Meson 未禁用锁定功能：${feature}`);
  }
  for (const feature of lock.features.disabled_boolean_options) {
    assert(options.get(feature) === false, `Meson 未禁用布尔功能：${feature}`);
  }

  const runtimeHashes = Object.fromEntries(await Promise.all(Object.entries(RUNTIME_FILES).map(async ([role, name]) => [role, hashBytes(await readFile(join(evidenceRoot, name)))])));
  const rows = [
    ...lock.sources.map((source) => ({
      kind: 'source', name: source.name, version: sourceVersion(source), usage: source.usage,
      license: source.license_expression, sha256: source.sha256, cache_path: source.cache_path,
    })),
    ...Object.entries(lock.toolchain.tools).sort(([left], [right]) => left.localeCompare(right)).map(([name, tool]) => ({
      kind: 'tool', name, version: tool.version, usage: 'build', license: tool.license_expression,
      sha256: tool.sha256 ?? '-', cache_path: tool.cache_path ?? 'builder-image',
    })),
  ];
  exactJson(
    parseTsv(await readFile(join(evidenceRoot, 'license-inventory.txt'), 'utf8'), ['kind', 'name', 'version', 'usage', 'license', 'sha256', 'cache_path'], 'license inventory'),
    rows, 'license inventory',
  );

  const cdx = await jsonFile(join(evidenceRoot, 'sbom.cdx.json'));
  assert(cdx.bomFormat === 'CycloneDX' && cdx.specVersion === '1.6', 'CycloneDX 标识无效');
  const cdxByRef = new Map(cdx.components.map((component) => [component['bom-ref'], component]));
  for (const source of lock.sources) {
    const component = cdxByRef.get(sourceRef(source));
    assert(component?.licenses?.[0]?.expression === source.license_expression, `CycloneDX 许可证不匹配：${source.name}`);
    assert(component?.hashes?.[0]?.content === source.sha256, `CycloneDX 输入哈希不匹配：${source.name}`);
    assert(propertiesMap(component).get('autolive:usage') === source.usage, `CycloneDX scope 不匹配：${source.name}`);
  }
  for (const [name, tool] of Object.entries(lock.toolchain.tools)) {
    const component = cdxByRef.get(toolRef(name, tool));
    assert(component?.licenses?.[0]?.expression === tool.license_expression, `CycloneDX 工具许可证不匹配：${name}`);
    assert(propertiesMap(component).get('autolive:usage') === 'build', `CycloneDX 工具 scope 不匹配：${name}`);
    if (tool.sha256) assert(component?.hashes?.[0]?.content === tool.sha256, `CycloneDX 工具输入哈希不匹配：${name}`);
  }
  for (const [role] of Object.entries(RUNTIME_FILES)) {
    assert(cdxByRef.get(`artifact:${role}`)?.hashes?.[0]?.content === runtimeHashes[role], `CycloneDX 产物哈希不匹配：${role}`);
    const edge = cdx.dependencies.find((item) => item.ref === `artifact:${role}`);
    exactJson(edge?.dependsOn, lock.sources.filter((source) => source.runtime_artifacts.includes(role)).map(sourceRef).sort(), `CycloneDX 依赖边：${role}`);
  }

  const spdx = await jsonFile(join(evidenceRoot, 'sbom.spdx.json'));
  assert(spdx.spdxVersion === 'SPDX-2.3', 'SPDX 标识无效');
  const spdxById = new Map(spdx.packages.map((item) => [item.SPDXID, item]));
  for (const source of lock.sources) {
    const item = spdxById.get(spdxId(sourceRef(source)));
    assert(item?.licenseDeclared === source.license_expression, `SPDX 许可证不匹配：${source.name}`);
    assert(item?.externalRefs?.[0]?.referenceLocator === source.usage, `SPDX scope 不匹配：${source.name}`);
  }
  for (const [name, tool] of Object.entries(lock.toolchain.tools)) {
    const item = spdxById.get(spdxId(toolRef(name, tool)));
    assert(item?.licenseDeclared === tool.license_expression, `SPDX 工具许可证不匹配：${name}`);
    assert(item?.externalRefs?.[0]?.referenceLocator === 'build', `SPDX 工具 scope 不匹配：${name}`);
  }
  for (const [role] of Object.entries(RUNTIME_FILES)) {
    const item = spdxById.get(`SPDXRef-Artifact-${role}`);
    assert(item?.checksums?.[0]?.checksumValue === runtimeHashes[role], `SPDX 产物哈希不匹配：${role}`);
    const expected = lock.sources.filter((source) => source.runtime_artifacts.includes(role)).map((source) => spdxId(sourceRef(source))).sort();
    const actual = spdx.relationships.filter((row) => row.spdxElementId === `SPDXRef-Artifact-${role}` && row.relationshipType === 'GENERATED_FROM').map((row) => row.relatedSpdxElement).sort();
    exactJson(actual, expected, `SPDX 依赖边：${role}`);
  }
}

async function verifyPeEvidence(lock, evidenceRoot) {
  const pe = await jsonFile(join(evidenceRoot, 'pe-imports.json'));
  assert(pe.schemaVersion === 2, 'PE imports schema 无效');
  exactJson(pe.bundled, lock.output_policy.bundled_dynamic_dependencies, 'PE bundled map');
  const observed = {};
  for (const [role, filename] of Object.entries(RUNTIME_FILES)) {
    const parsed = parsePe(await readFile(join(evidenceRoot, filename)), filename);
    assert(parsed.subsystem === 2 || parsed.subsystem === 3, `${filename} subsystem 无效`);
    observed[filename] = parsed.imports;
    if (role === 'mpv-executable') {
      for (const dll of Object.keys(lock.output_policy.bundled_dynamic_dependencies)) {
        assert(parsed.imports.includes(dll), `mpv.exe 未硬导入随包 DLL：${dll}`);
      }
    }
  }
  exactJson(pe.artifacts, observed, 'PE imports 与真实二进制');
  const union = [...new Set(Object.values(observed).flat())].sort();
  exactJson(pe.imports, union, 'PE import union');
  assert(union.every((dll) => lock.output_policy.dynamic_dependencies_allowlist.includes(dll)), 'PE imports 超出锁定允许列表');
}

export function verifyDockerHostEvidence({ lock, lockBytes, evidence }) {
  const lockHash = hashBytes(lockBytes);
  assert(evidence.schemaVersion === 2 && evidence.claim === 'one_locked_cold_build_candidate', 'Docker 宿主证据声明无效');
  assert(evidence.lockSha256 === lockHash, 'Docker 宿主证据锁哈希不匹配');
  assert(evidence.dockerBuild.network === 'none' && evidence.dockerBuild.pull === false, 'Docker 镜像构建必须断网且禁止 pull');
  assert(evidence.dockerBuild.context === 'desktop/third_party/mpv'
    && evidence.dockerBuild.dockerfile === 'desktop/third_party/mpv/build/Dockerfile', 'Docker 构建上下文或 Dockerfile 无效');
  assert(evidence.baseImage.reference === lock.toolchain.builder_image && /^sha256:[a-f0-9]{64}$/.test(evidence.baseImage.id), 'Docker 基础镜像证据无效');
  assert(Array.isArray(evidence.baseImage.repoDigests)
    && evidence.baseImage.repoDigests.includes(lock.toolchain.builder_image), 'Docker 基础镜像摘要未锁定');
  assert(/^sha256:[a-f0-9]{64}$/.test(evidence.recipeImage.id), 'Docker 配方镜像 ID 无效');
  assert(evidence.recipeImage.tag === `autolive-mpv-phase7a:${lockHash.slice(0, 16)}`, 'Docker 配方镜像标签与锁不一致');
  const volume = evidence.workVolume;
  assert(volume?.driver === 'local' && volume.scope === 'local', 'Docker work volume 必须是本机 local volume');
  assert(/^[a-f0-9]{32}$/.test(volume.runId) && volume.name === `autolive-phase7a-work-${volume.runId}`, 'Docker work volume 名称或随机运行 ID 无效');
  assert(volume.existedBeforeCreate === false && volume.retained === true, 'Docker work volume 必须全新创建并保留');
  assert(typeof volume.createdAt === 'string' && volume.createdAt.length > 0, 'Docker work volume 创建时间缺失');
  assert(volume.labels?.runId === volume.runId, 'Docker work volume run-id 标签不匹配');
  assert(volume.labels?.lockSha256 === lockHash, 'Docker work volume lock 标签不匹配');
  assert(volume.labels?.purpose === 'cold-build-work', 'Docker work volume purpose 标签无效');
  const container = evidence.container;
  assert(/^[a-f0-9]{64}$/.test(container.id) && container.networkMode === 'none', 'Docker 容器身份或网络模式无效');
  assert(container.name === `autolive-phase7a-${lockHash.slice(0, 12)}-${volume.runId.slice(0, 12)}`, 'Docker 容器名称与锁或 run-id 不一致');
  exactJson(container.entrypoint, ['/bin/bash', '/recipe/build.sh'], 'Docker entrypoint');
  exactJson(container.command, [null], 'Docker command');
  assert(container.user === '0:0', 'Docker volume 构建用户必须固定为 0:0');
  assert(container.memoryBytes === 4 * 1024 ** 3, 'Docker 容器内存上限必须固定为 4 GiB');
  assert(container.memorySwapBytes === container.memoryBytes,
    'Docker 容器 memory-swap 必须等于内存上限并禁止额外 swap');
  assert(container.readonlyRootfs === true && container.exitCode === 0 && container.dockerStartExitCode === 0 && container.status === 'exited', 'Docker 容器未成功只读退出');
  const startedAt = Date.parse(container.startedAt);
  const finishedAt = Date.parse(container.finishedAt);
  assert(Number.isFinite(startedAt) && Number.isFinite(finishedAt) && finishedAt >= startedAt, 'Docker 容器起止时间无效');
  assert(Array.isArray(container.mounts) && container.mounts.length === 3, 'Docker 容器挂载集合必须精确为三个');
  const mounts = new Map(container.mounts.map((mount) => [mount.destination, mount]));
  assert(mounts.size === 3, 'Docker 容器挂载目标不允许重复');
  const inputs = mounts.get('/build-inputs');
  const output = mounts.get('/out');
  const work = mounts.get('/work');
  assert(inputs?.type === 'bind' && inputs.readWrite === false && inputs.noCopy === false, 'Docker build-inputs 必须是只读 bind');
  assert(output?.type === 'bind' && output.readWrite === true && output.noCopy === false, 'Docker out 必须是可写 bind');
  assert(work?.type === 'volume' && work.readWrite === true && work.noCopy === true, 'Docker /work 必须是 nocopy 可写 volume');
  assert(work.name === volume.name, 'Docker /work 挂载未使用证据中的唯一 volume');
}

async function verifyDockerEvidence(lock, lockBytes, evidenceRoot) {
  const evidence = await jsonFile(join(evidenceRoot, 'docker-host-evidence.json'));
  verifyDockerHostEvidence({ lock, lockBytes, evidence });
}

export async function verifySemanticBuildEvidence({ lock, lockBytes, evidenceRoot, buildInputsRoot }) {
  await verifyStructuredEvidence({ lock, lockBytes, evidenceRoot });
  await verifyPeEvidence(lock, evidenceRoot);
  await verifyDockerEvidence(lock, lockBytes, evidenceRoot);
  const sourceRoot = await verifyPatchArchives({ lock, lockBytes, evidenceRoot, buildInputsRoot });
  try {
    await verifyCopyright(lock, evidenceRoot, sourceRoot);
  } finally {
    await cleanupTemporary(sourceRoot);
  }
  const log = await readFile(join(evidenceRoot, 'build-log.txt'), 'utf8');
  assert(/Linking target mpv\.exe/.test(log), '构建日志缺少 mpv.exe 链接证据');
  return { semanticEvidenceVerified: true, claim: 'one_locked_cold_build_candidate' };
}
