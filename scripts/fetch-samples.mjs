// Downloads the sample corpus listed in samples/manifest.json into samples/corpus/.
//
//   node scripts/fetch-samples.mjs            # everything
//   node scripts/fetch-samples.mjs barrel_03  # only the named assets
//
// Files already present with the right checksum are skipped.

import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, normalize, isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(await readFile(join(root, "samples", "manifest.json"), "utf8"));
const only = new Set(process.argv.slice(2));
const assets = manifest.assets.filter((a) => only.size === 0 || only.has(a.id));
const headers = { "User-Agent": "LittleMesh sample fetcher" };

async function md5Matches(path, md5) {
  try {
    return createHash("md5").update(await readFile(path)).digest("hex") === md5;
  } catch {
    return false;
  }
}

async function download(url, path, md5) {
  if (await md5Matches(path, md5)) return 0;
  const response = await fetch(url, { headers });
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const data = Buffer.from(await response.arrayBuffer());
  if (createHash("md5").update(data).digest("hex") !== md5) throw new Error(`${url}: checksum mismatch`);
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, data);
  return data.length;
}

async function fetchAsset({ id }) {
  const response = await fetch(`https://api.polyhaven.com/files/${id}`, { headers });
  if (!response.ok) throw new Error(`${id}: file list HTTP ${response.status}`);
  const entry = (await response.json()).gltf?.[manifest.resolution]?.gltf;
  if (!entry) throw new Error(`${id}: no ${manifest.resolution} glTF available`);

  const dir = join(root, "samples", "corpus", id);
  let bytes = await download(entry.url, join(dir, `${id}.gltf`), entry.md5);
  for (const [relative, file] of Object.entries(entry.include ?? {})) {
    // Never write outside the asset's folder.
    const safe = normalize(relative);
    if (isAbsolute(safe) || safe.startsWith("..")) throw new Error(`${id}: unsafe path ${relative}`);
    bytes += await download(file.url, join(dir, safe), file.md5);
  }
  return bytes;
}

let total = 0;
let failed = 0;
const queue = [...assets];
async function worker() {
  for (let asset = queue.shift(); asset; asset = queue.shift()) {
    try {
      const bytes = await fetchAsset(asset);
      total += bytes;
      console.log(`${bytes ? "fetched" : "up to date"}  ${asset.id}${bytes ? ` (${(bytes / 1e6).toFixed(1)} MB)` : ""}`);
    } catch (error) {
      failed += 1;
      console.error(`failed     ${asset.id}: ${error.message}`);
    }
  }
}
await Promise.all(Array.from({ length: 4 }, worker));
console.log(`\n${assets.length - failed}/${assets.length} assets ready, ${(total / 1e6).toFixed(1)} MB downloaded`);
process.exit(failed ? 1 : 0);
