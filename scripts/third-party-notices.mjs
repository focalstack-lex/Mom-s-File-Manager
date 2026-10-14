/**
 * Writes THIRD-PARTY-NOTICES.txt: the license notices of every open source
 * package compiled into Tote, as their licenses (MIT, ISC, Apache-2.0, BSD,
 * SIL OFL and others) require when the software is redistributed.
 *
 *   node scripts/third-party-notices.mjs
 *
 * Rust: `cargo metadata` for x86_64-pc-windows-msvc, following normal
 * dependencies only (build scripts and dev tools are not shipped).
 * npm: the production dependencies in package.json and what they depend on.
 * Identical license texts are printed once, after the packages that use them.
 * The file is bundled next to tote.exe (tauri.conf.json bundle.resources) and
 * published at codewithlex.site/tote/third-party-notices.txt by release.mjs.
 */

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "THIRD-PARTY-NOTICES.txt");
const LICENSE_FILE = /^(licen[cs]e|copying|notice|copyright|unlicense)([-._].*)?$/i;

function licenseTexts(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isFile() && LICENSE_FILE.test(e.name))
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((e) => readFileSync(join(dir, e.name), "utf8").replace(/\r\n/g, "\n").trim())
    .filter(Boolean);
}

// Rust packages reachable from tote through normal dependencies.
function rustPackages() {
  const meta = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--filter-platform", "x86_64-pc-windows-msvc", "--manifest-path", join(ROOT, "src-tauri", "Cargo.toml")], {
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
    }),
  );
  const byId = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const rootId = meta.resolve.root;
  const seen = new Set([rootId]);
  const queue = [rootId];
  while (queue.length) {
    const node = nodes.get(queue.shift());
    for (const dep of node?.deps ?? []) {
      const normal = dep.dep_kinds.some((k) => k.kind === null);
      if (normal && !seen.has(dep.pkg)) {
        seen.add(dep.pkg);
        queue.push(dep.pkg);
      }
    }
  }
  seen.delete(rootId);
  return [...seen].map((id) => byId.get(id)).map((p) => ({
    name: p.name,
    version: p.version,
    license: p.license ?? (p.license_file ? `see ${p.license_file}` : "not stated"),
    source: p.repository ?? p.homepage ?? `https://crates.io/crates/${p.name}`,
    texts: licenseTexts(dirname(p.manifest_path)),
  }));
}

// npm packages bundled into the window: production dependencies, recursively.
function npmPackages() {
  const rootPkg = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8"));
  const found = new Map();
  const queue = Object.keys(rootPkg.dependencies ?? {});
  while (queue.length) {
    const name = queue.shift();
    if (found.has(name)) continue;
    const dir = join(ROOT, "node_modules", name);
    const manifest = join(dir, "package.json");
    if (!existsSync(manifest)) continue;
    const pkg = JSON.parse(readFileSync(manifest, "utf8"));
    const repo = typeof pkg.repository === "string" ? pkg.repository : pkg.repository?.url;
    found.set(name, {
      name,
      version: pkg.version,
      license: typeof pkg.license === "string" ? pkg.license : pkg.license?.type ?? "not stated",
      source: repo ?? pkg.homepage ?? `https://www.npmjs.com/package/${name}`,
      texts: licenseTexts(dir),
    });
    queue.push(...Object.keys(pkg.dependencies ?? {}));
  }
  return [...found.values()];
}

function render(title, packages) {
  packages.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
  const groups = new Map();
  const withoutText = [];
  for (const p of packages) {
    if (p.texts.length === 0) {
      withoutText.push(p);
      continue;
    }
    const key = p.texts.join("\n\n-----\n\n");
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(p);
  }
  const lines = [`${"=".repeat(78)}`, title, `${packages.length} packages`, `${"=".repeat(78)}`, ""];
  for (const [text, users] of groups) {
    lines.push(...users.map((p) => `${p.name} ${p.version} (${p.license}) ${p.source}`), "", text, "", "-".repeat(78), "");
  }
  if (withoutText.length) {
    lines.push("These packages ship no license file of their own; their license is the one named, available at the source given:", "");
    lines.push(...withoutText.map((p) => `${p.name} ${p.version} (${p.license}) ${p.source}`), "");
  }
  return lines.join("\n");
}

const rust = rustPackages();
const npm = npmPackages();
const header = [
  "Tote third-party notices",
  "",
  "Tote is MIT-licensed software by Lex Matondo (see LICENSE.txt). It includes the",
  "open source packages below, each under its own license. Their copyright",
  "notices and license texts follow, as those licenses require.",
  "",
  "Generated by scripts/third-party-notices.mjs from the exact dependency graph",
  `of Tote ${JSON.parse(readFileSync(join(ROOT, "src-tauri", "tauri.conf.json"), "utf8")).version}.`,
  "",
].join("\n");
writeFileSync(OUT, `${header}\n${render("Window (JavaScript) packages", npm)}\n${render("Native (Rust) packages", rust)}`);
const missing = [...rust, ...npm].filter((p) => p.texts.length === 0).length;
process.stdout.write(`THIRD-PARTY-NOTICES.txt: ${npm.length} npm and ${rust.length} Rust packages, ${missing} without a bundled license file, ${Math.round(readFileSync(OUT).length / 1024)} KB\n`);
