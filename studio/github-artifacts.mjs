import { inflateRawSync, gunzipSync } from "node:zlib";
import { createHash } from "node:crypto";
import { StudioError } from "./store.mjs";
const fail = () => { throw new StudioError("빌드 결과물의 형식·무결성을 확인할 수 없습니다."); };
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const maxSize = 256 * 1024 * 1024;
const safeNumber = (value) => { const n = Number(value); if (!Number.isSafeInteger(n) || n < 0) fail(); return n; };
function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) { crc ^= byte; for (let i = 0; i < 8; i++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1)); }
  return (crc ^ 0xffffffff) >>> 0;
}
// Read the two expected files in GitHub's ZIP wrapper. Never extract paths to disk.
export function artifactFiles(zip, expectedNames) {
  let end = -1;
  for (let i = zip.length - 22; i >= Math.max(0, zip.length - 65557); i--)
    if (zip.readUInt32LE(i) === 0x06054b50 && i + 22 + zip.readUInt16LE(i + 20) === zip.length) { end = i; break; }
  if (end < 0) fail();
  let count = zip.readUInt16LE(end + 10), offset = zip.readUInt32LE(end + 16);
  if (count === 65535 || offset === 0xffffffff) {
    if (end < 20 || zip.readUInt32LE(end - 20) !== 0x07064b50) fail();
    const record = safeNumber(zip.readBigUInt64LE(end - 12));
    if (record + 56 > zip.length || zip.readUInt32LE(record) !== 0x06064b50) fail();
    count = safeNumber(zip.readBigUInt64LE(record + 32)); offset = safeNumber(zip.readBigUInt64LE(record + 48));
  }
  if (count < 1 || count > 8) fail();
  const result = new Map(); let total = 0;
  for (let i = 0; i < count; i++) {
    if (offset + 46 > zip.length || zip.readUInt32LE(offset) !== 0x02014b50) fail();
    const flags = zip.readUInt16LE(offset + 8), method = zip.readUInt16LE(offset + 10), crc = zip.readUInt32LE(offset + 16);
    let compressed = zip.readUInt32LE(offset + 20), size = zip.readUInt32LE(offset + 24), local = zip.readUInt32LE(offset + 42);
    const nameLength = zip.readUInt16LE(offset + 28), extraLength = zip.readUInt16LE(offset + 30), commentLength = zip.readUInt16LE(offset + 32);
    if (offset + 46 + nameLength + extraLength + commentLength > zip.length || flags & 1) fail();
    const name = zip.subarray(offset + 46, offset + 46 + nameLength).toString("utf8");
    if (!expectedNames.includes(name) || result.has(name)) fail();
    const extra = zip.subarray(offset + 46 + nameLength, offset + 46 + nameLength + extraLength);
    for (let at = 0; at + 4 <= extra.length;) {
      const type = extra.readUInt16LE(at), len = extra.readUInt16LE(at + 2); at += 4;
      if (at + len > extra.length) fail();
      if (type === 1) {
        let p = at;
        const next = () => { if (p + 8 > at + len) fail(); const n = safeNumber(extra.readBigUInt64LE(p)); p += 8; return n; };
        if (size === 0xffffffff) size = next(); if (compressed === 0xffffffff) compressed = next(); if (local === 0xffffffff) local = next();
      }
      at += len;
    }
    total += size;
    if (total > maxSize || local + 30 > zip.length || zip.readUInt32LE(local) !== 0x04034b50) fail();
    const localNameLength = zip.readUInt16LE(local + 26);
    const start = local + 30 + localNameLength + zip.readUInt16LE(local + 28);
    if (zip.readUInt16LE(local + 6) !== flags || zip.readUInt16LE(local + 8) !== method ||
        zip.subarray(local + 30, local + 30 + localNameLength).toString("utf8") !== name) fail();
    if (start + compressed > zip.length || ![0, 8].includes(method)) fail();
    const data = zip.subarray(start, start + compressed);
    const bytes = method === 0 ? Buffer.from(data) : inflateRawSync(data, { maxOutputLength: Math.max(1, size) });
    if (bytes.length !== size || crc32(bytes) !== crc) fail();
    result.set(name, bytes);
    offset += 46 + nameLength + extraLength + commentLength;
  }
  if (result.size !== expectedNames.length) fail();
  return result;
}
function tarReports(archive) {
  const tar = gunzipSync(archive, { maxOutputLength: maxSize });
  const reports = []; let pendingPath;
  for (let at = 0; at + 512 <= tar.length;) {
    const h = tar.subarray(at, at + 512);
    if (h.every((n) => n === 0)) break;
    let name = h.subarray(0, 100).toString("utf8").split("\0")[0];
    const prefix = h.subarray(345, 500).toString("utf8").split("\0")[0];
    if (prefix) name = prefix + "/" + name;
    const raw = h.subarray(124, 136).toString("ascii").replace(/\0/g, "").trim();
    if (!/^[0-7]+$/.test(raw)) fail();
    const size = parseInt(raw, 8), start = at + 512;
    if (start + size > tar.length) fail();
    if (h[156] === 120 || h[156] === 76) {
      if (size > 1024 * 1024) fail();
      const data = tar.subarray(start, start + size);
      if (h[156] === 76) pendingPath = data.toString("utf8").replace(/\0.*$/s, "");
      else for (let p = 0; p < data.length;) {
        const space = data.indexOf(32, p), length = Number(data.subarray(p, space).toString("ascii"));
        if (space < p || !Number.isInteger(length) || length <= space - p + 1 || p + length > data.length) fail();
        const record = data.subarray(space + 1, p + length - 1).toString("utf8");
        if (record.startsWith("path=")) pendingPath = record.slice(5);
        p += length;
      }
      at = start + Math.ceil(size / 512) * 512; continue;
    }
    if (pendingPath) { name = pendingPath; pendingPath = undefined; }
    if ((name === "source.json" || name.endsWith("/build-report.json")) && [0, 48].includes(h[156])) {
      if (size > 4 * 1024 * 1024) fail();
      reports.push({ name, value: JSON.parse(tar.subarray(start, start + size).toString("utf8")) });
    }
    at = start + Math.ceil(size / 512) * 512;
  }
  return reports;
}
export function verifyArtifact(zip, artifact, job, target) {
  if (!/^sha256:[a-f0-9]{64}$/.test(artifact.digest || "") || artifact.digest !== `sha256:${sha256(zip)}`) fail();
  const archiveName = `project-${target}.tar.gz`, receiptName = `receipt-${target}.json`;
  const files = artifactFiles(zip, [archiveName, receiptName]);
  const archive = files.get(archiveName), receipt = JSON.parse(files.get(receiptName));
  if (receipt.jobId !== job.id || receipt.target !== target || receipt.projectHash !== job.contentHash || receipt.engineHash !== job.github.engineHash ||
      receipt.identifier !== job.projectId || receipt.sha256 !== sha256(archive) || receipt.bytes !== archive.length) fail();
  const reports = tarReports(archive), sources = reports.filter((r) => r.name === "source.json"), builds = reports.filter((r) => r.name.endsWith("/build-report.json"));
  if (sources.length !== 1 || builds.length !== 1) fail();
  const source = sources[0].value, report = builds[0].value;
  if (source.contentHash !== job.contentHash || source.identifier !== job.projectId || report.projectHash !== job.contentHash ||
      report.identifier !== job.projectId || report.target !== target || report.development !== true || report.staticInspection !== "passed") fail();
  return { archive, archiveName, receipt: files.get(receiptName), receiptName, report };
}
