import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export const DEFAULT_BASE_URL = 'http://101.96.208.132:7088/autolive-resources/v0.1.0/';
const RELEASE = 'v0.1.0';
const SHA256 = /^[a-f0-9]{64}$/;

function assertSafeRelativePath(relativePath) {
	if (typeof relativePath !== 'string' || relativePath.length === 0 || relativePath.includes('\\')) {
		throw new Error('manifest relative_path is invalid');
	}
	const parts = relativePath.split('/');
	if (
		parts.some((part) => part.length === 0 || part === '.' || part === '..' || part.startsWith('.')) ||
		/^[A-Za-z]:/.test(parts[0])
	) {
		throw new Error('manifest relative_path is not safe');
	}
}

function assertManifest(manifest) {
	if (!manifest || manifest.schema_version !== 1 || manifest.release !== RELEASE || !Array.isArray(manifest.files)) {
		throw new Error('runtime resource manifest is invalid');
	}
	for (const file of manifest.files) {
		if (!file || typeof file !== 'object') throw new Error('runtime resource file entry is invalid');
		assertSafeRelativePath(file.relative_path);
		if (!Number.isSafeInteger(file.size_bytes) || file.size_bytes < 0) {
			throw new Error('runtime resource file size is invalid');
		}
		if (typeof file.sha256 !== 'string' || !SHA256.test(file.sha256)) {
			throw new Error('runtime resource file sha256 is invalid');
		}
	}
}

function resourceUrl(baseUrl, relativePath) {
	assertSafeRelativePath(relativePath);
	const base = new URL(baseUrl.endsWith('/') ? baseUrl : `${baseUrl}/`);
	if (base.protocol !== 'http:' && base.protocol !== 'https:') throw new Error('base URL protocol is invalid');
	const encodedPath = relativePath.split('/').map((part) => encodeURIComponent(part)).join('/');
	return new URL(encodedPath, base);
}

async function request(url, options = {}) {
	return fetch(url, { ...options, redirect: 'error' });
}

function contentLength(response) {
	const value = response.headers.get('content-length');
	if (!value || !/^\d+$/.test(value)) throw new Error('response Content-Length is missing or invalid');
	return Number(value);
}

async function hashResponse(response, expectedSize) {
	if (!response.body) throw new Error('GET response body is missing');
	const hash = createHash('sha256');
	let size = 0;
	for await (const chunk of response.body) {
		size += chunk.byteLength;
		if (size > expectedSize) throw new Error('GET response is larger than manifest');
		hash.update(chunk);
	}
	return { size, sha256: hash.digest('hex') };
}

export async function verifyResourceServer({ baseUrl = DEFAULT_BASE_URL, manifest, sampleRelativePath } = {}) {
	assertManifest(manifest);
	const file = manifest.files.find(({ relative_path }) => relative_path === sampleRelativePath) ?? manifest.files[0];
	if (!file) throw new Error('runtime resource manifest has no files');
	if (file.size_bytes < 16) throw new Error('sample resource must contain at least 16 bytes');
	const url = resourceUrl(baseUrl, file.relative_path);

	const headResponse = await request(url, { method: 'HEAD' });
	if (headResponse.status !== 200 || contentLength(headResponse) !== file.size_bytes) {
		throw new Error(`HEAD verification failed: HTTP ${headResponse.status}`);
	}

	const getResponse = await request(url, { method: 'GET' });
	if (getResponse.status !== 200) throw new Error(`GET verification failed: HTTP ${getResponse.status}`);
	const contentLengthHeader = contentLength(getResponse);
	const digest = await hashResponse(getResponse, file.size_bytes);
	if (contentLengthHeader !== file.size_bytes || digest.size !== file.size_bytes || digest.sha256 !== file.sha256) {
		throw new Error('GET resource size or sha256 verification failed');
	}

	const rangeResponse = await request(url, { method: 'GET', headers: { Range: 'bytes=0-15' } });
	const rangeBody = new Uint8Array(await rangeResponse.arrayBuffer());
	if (
		rangeResponse.status !== 206 ||
		rangeResponse.headers.get('content-range') !== `bytes 0-15/${file.size_bytes}` ||
		rangeBody.byteLength !== 16
	) {
		throw new Error('valid Range verification failed');
	}

	const invalidRangeResponse = await request(url, {
		method: 'GET',
		headers: { Range: `bytes=${file.size_bytes}-` },
	});
	if (invalidRangeResponse.status !== 416) throw new Error('out-of-bounds Range was not rejected');

	return { head: true, get: true, range: true, hash: true };
}

async function main() {
	const [manifestPath, baseUrl] = process.argv.slice(2);
	if (!manifestPath || process.argv.length > 4) {
		throw new Error('usage: verify-server.mjs <manifest-path> [base-url]');
	}
	const manifest = JSON.parse(readFileSync(resolve(manifestPath), 'utf8'));
	const result = await verifyResourceServer({ baseUrl: baseUrl ?? DEFAULT_BASE_URL, manifest });
	console.log(`HEAD PASS\nGET PASS\nRange PASS\nSHA-256 PASS\n${JSON.stringify(result)}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
	main().catch((error) => {
		console.error(error instanceof Error ? error.message : String(error));
		process.exitCode = 1;
	});
}
