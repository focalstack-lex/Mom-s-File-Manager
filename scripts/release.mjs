/**
 * Tote release: build a signed installer and publish it to the download page.
 *
 *   npm run release -- --notes "What changed, in one plain sentence."
 *   npm run release -- --skip-build --notes "..."   (publish the last build)
 *
 * 1. Builds with the updater key (TOTE_UPDATER_KEY, default
 *    %USERPROFILE%\.tauri\tote-updater.key), so the bundle gets a .sig file.
 *    The key never leaves that folder and is never printed.
 * 2. Copies Tote_<version>_x64-setup.exe into the portfolio's tote/download/
 *    (TOTE_SITE_DIR, default C:/Users/User/Documents/Portfolio_Lex/tote),
 *    removes older installers there, and writes tote/latest.json, which every
 *    installed Tote reads once a day.
 * 3. Updates the file name, version, size and SHA-256 on tote/index.html.
 *
 * It stops before committing or deploying: review the portfolio diff, commit,
 * then run `vercel deploy --prod` in the portfolio folder.
 */

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, rmSync, statSync, copyFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SITE = resolve(process.env.TOTE_SITE_DIR ?? "C:/Users/User/Documents/Portfolio_Lex/tote");
const KEY = resolve(process.env.TOTE_UPDATER_KEY ?? join(homedir(), ".tauri", "tote-updater.key"));
const PUBLIC_BASE = "https://www.codewithlex.site/tote";

function fail(message) {
  process.stderr.write(`[RELEASE FAIL] ${message}\n`);
  process.exit(1);
}

const args = process.argv.slice(2);
const notesAt = args.indexOf("--notes");
const notes = notesAt >= 0 ? args[notesAt + 1] : undefined;
if (!notes || notes.startsWith("--")) fail('add --notes "What changed, in one plain sentence." (she sees it as the update description)');
if (/[\u2013\u2014]/.test(notes)) fail("the notes contain an en or em dash; use a comma, colon or full stop");
const skipBuild = args.includes("--skip-build");

const version = JSON.parse(readFileSync(join(ROOT, "src-tauri", "tauri.conf.json"), "utf8")).version;
const exeName = `Tote_${version}_x64-setup.exe`;
const bundleDir = join(ROOT, "src-tauri", "target", "release", "bundle", "nsis");
const exePath = join(bundleDir, exeName);
const sigPath = `${exePath}.sig`;

if (!existsSync(join(SITE, "index.html"))) fail(`no download page at ${SITE} (set TOTE_SITE_DIR)`);

if (!skipBuild) {
  if (!existsSync(KEY)) fail(`no updater key at ${KEY}. Without it no installed Tote will accept the update.`);
  if (ROOT.includes("'") && !process.env.TAURI_CONFIG) fail("this checkout path has an apostrophe; set TAURI_CONFIG as described in README (Building on a path with an apostrophe)");
  process.stdout.write(`Building Tote ${version} (signed)...\n`);
  const build = spawnSync("npx", ["tauri", "build"], {
    cwd: ROOT,
    stdio: "inherit",
    shell: true,
    env: { ...process.env, TAURI_SIGNING_PRIVATE_KEY: KEY, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "" },
  });
  if (build.status !== 0) fail(`tauri build exited with ${build.status}`);
}

if (!existsSync(exePath)) fail(`no installer at ${exePath}; build first`);
if (!existsSync(sigPath)) fail(`no signature at ${sigPath}; the build ran without the updater key`);

const bytes = readFileSync(exePath);
const sha256 = createHash("sha256").update(bytes).digest("hex");
const size = statSync(exePath).size;
const sizeLabel = `${(size / 1024 / 1024).toFixed(1)} MB`;

// 1. The installer: replace older ones so the folder only holds what the page links.
const downloadDir = join(SITE, "download");
for (const old of readdirSync(downloadDir).filter((f) => /^Tote_.*_x64-setup\.exe$/.test(f) && f !== exeName)) {
  rmSync(join(downloadDir, old));
  process.stdout.write(`removed ${old}\n`);
}
copyFileSync(exePath, join(downloadDir, exeName));

// 2. The manifest every installed Tote reads.
const manifest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature: readFileSync(sigPath, "utf8").trim(),
      url: `${PUBLIC_BASE}/download/${exeName}`,
    },
  },
};
writeFileSync(join(SITE, "latest.json"), `${JSON.stringify(manifest, null, 2)}\n`);

// 3. The page: every place that names the file, version, size or checksum.
const pagePath = join(SITE, "index.html");
let page = readFileSync(pagePath, "utf8");
const swaps = [
  [/Tote_\d+\.\d+\.\d+_x64-setup\.exe/g, exeName, "installer name"],
  [/version \d+\.\d+\.\d+/g, `version ${version}`, "version"],
  [/\d+(?:\.\d+)? MB/g, sizeLabel, "size"],
  [/\b[0-9a-f]{64}\b/g, sha256, "SHA-256"],
];
for (const [pattern, value, label] of swaps) {
  if (!pattern.test(page)) fail(`index.html has no ${label} to update; check the page by hand`);
  page = page.replace(pattern, value);
}
writeFileSync(pagePath, page);

process.stdout.write(
  [
    "",
    `[RELEASE READY] Tote ${version}`,
    `  installer  ${join(downloadDir, exeName)} (${sizeLabel}, ${size} bytes)`,
    `  SHA-256    ${sha256}`,
    `  manifest   ${join(SITE, "latest.json")}`,
    "Next: review the portfolio diff, commit, then run `vercel deploy --prod` in the portfolio folder.",
    "Installed copies offer the update within a day of their last check.",
    "",
  ].join("\n"),
);
