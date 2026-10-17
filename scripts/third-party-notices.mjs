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
 * A package that ships no license file of its own takes its text from
 * licenses/<name>/ (licenses/README.md says where each came from), and the
 * WebView2 loader webview2-com-sys links into tote.exe gets Microsoft's
 * notice from licenses/webview2-loader/. When a shipped package has a text in
 * neither place, the script names it and fails without writing the file
 * (audit T-39).
 * The file is bundled next to tote.exe (tauri.conf.json bundle.resources) and
 * published at codewithlex.site/tote/third-party-notices.txt by release.mjs.
 */

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "THIRD-PARTY-NOTICES.txt");
const LICENSES = join(ROOT, "licenses");
const LICENSE_FILE = /^(licen[cs]e|copying|notice|copyright|unlicense)([-._].*)?$/i;

// Linked into tote.exe without a package of its own. `checkedFor` is the
// version of the crate that links it whose SDK was looked up; any other
// version fails the script until it is looked up again (licenses/README.md).
const LINKED = [
  {
    via: "webview2-com-sys",
    checkedFor: "0.39.1",
    name: "Microsoft WebView2 loader (WebView2LoaderStatic.lib)",
    version: "1.0.3800.47",
    license: "Microsoft WebView2 SDK license",
    source: "https://www.nuget.org/packages/Microsoft.Web.WebView2/1.0.3800.47",
    folder: "webview2-loader",
  },
];

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

/** Packages with no license file of their own take the texts in licenses/<name>/; their own file always wins. */
export function withVendoredTexts(packages, dir = LICENSES) {
  return packages.map((p) => (p.texts.length ? p : { ...p, texts: licenseTexts(join(dir, p.name)), vendored: true }));
}

/** The packages the notices would list without any license text. */
export function packagesWithoutText(packages) {
  return packages.filter((p) => p.texts.length === 0);
}

/** The LINKED components the Rust graph ships, and a problem for each whose crate changed version. */
function linkedComponents(rust) {
  const linked = [];
  const problems = [];
  for (const item of LINKED) {
    const via = rust.find((p) => p.name === item.via);
    if (!via) continue;
    if (via.version !== item.checkedFor) {
      problems.push(
        `${item.via} is ${via.version} now, but licenses/${item.folder}/ was checked for ${item.checkedFor}; look up the WebView2 SDK it links, compare its license and update LINKED in this script (licenses/README.md)`,
      );
      continue;
    }
    linked.push({
      name: item.name,
      version: item.version,
      license: `${item.license}, linked by ${item.via} ${via.version}`,
      source: item.source,
      texts: licenseTexts(join(LICENSES, item.folder)),
    });
  }
  return { linked, problems };
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

// Every package has a text by now: main() stops before rendering when one has none.
function render(title, packages, unit = "packages") {
  packages.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
  const groups = new Map();
  for (const p of packages) {
    const key = p.texts.join("\n\n-----\n\n");
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(p);
  }
  const count = `${packages.length} ${packages.length === 1 ? unit.replace(/s$/, "") : unit}`;
  const lines = [`${"=".repeat(78)}`, title, count, `${"=".repeat(78)}`, ""];
  for (const [text, users] of groups) {
    lines.push(...users.map((p) => `${p.name} ${p.version} (${p.license}) ${p.source}`), "", text, "", "-".repeat(78), "");
  }
  return lines.join("\n");
}

function main() {
  const rust = withVendoredTexts(rustPackages());
  const npm = withVendoredTexts(npmPackages());
  const { linked, problems } = linkedComponents(rust);
  for (const p of packagesWithoutText([...npm, ...rust, ...linked])) {
    problems.push(`${p.name} ${p.version} (${p.license}) ships no license text and licenses/${p.name}/ has none; add its upstream text there (licenses/README.md)`);
  }
  if (problems.length) {
    for (const problem of problems) process.stderr.write(`[NOTICES FAIL] ${problem}\n`);
    process.stderr.write("THIRD-PARTY-NOTICES.txt was not written.\n");
    process.exit(1);
  }
  const used = new Set([...rust, ...npm].filter((p) => p.vendored).map((p) => p.name).concat(LINKED.map((item) => item.folder)));
  for (const entry of existsSync(LICENSES) ? readdirSync(LICENSES, { withFileTypes: true }) : []) {
    if (entry.isDirectory() && !used.has(entry.name)) process.stdout.write(`licenses/${entry.name}/ is not used by any shipped package now; remove it\n`);
  }
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
  const sections = [render("Window (JavaScript) packages", npm), render("Native (Rust) packages", rust)];
  if (linked.length) sections.push(render("Linked into tote.exe without a package of its own", linked, "components"));
  writeFileSync(OUT, `${header}\n${sections.join("\n")}`);
  const vendored = [...rust, ...npm].filter((p) => p.vendored).length;
  process.stdout.write(
    `THIRD-PARTY-NOTICES.txt: ${npm.length} npm and ${rust.length} Rust packages (${vendored} with a text from licenses/), ${linked.length} linked component(s), ${Math.round(readFileSync(OUT).length / 1024)} KB\n`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
