/**
 * Tote release: build a signed installer and publish it to the download page.
 *
 *   npm run release -- --notes "What changed, in one plain sentence."
 *   npm run release -- --skip-build --notes "..."   (publish the last build)
 *   npm run release -- --key-change <old public key file> --notes "..."
 *       (only the one release that moves installed copies to a new updater
 *       key; README, The updater key)
 *
 * 1. Checks everything before it writes anything (README, Release preflight):
 *    the notes; one version in package.json, tauri.conf.json and Cargo.toml,
 *    higher than the published latest.json and the portfolio's copy; this
 *    checkout and the portfolio both committed; no tag v<version> yet; a
 *    download page that names one installer and one checksum; and for a
 *    build, the updater key (TOTE_UPDATER_KEY, default
 *    %USERPROFILE%\.tauri\tote-updater.key) and a non-empty passphrase in
 *    TAURI_SIGNING_PRIVATE_KEY_PASSWORD.
 * 2. Builds with the key kept out of the environment (the build runs
 *    `npm run build` and every package that pulls in), then signs the
 *    installer in a step of its own with `tauri signer sign`, the only
 *    process that is given the key path and the passphrase. The key never
 *    leaves its folder and is never printed.
 * 3. Verifies the .sig against the public key installed copies trust and
 *    checks that its trusted comment names this version and file, the way
 *    the updater on her laptop does; plans the page change in memory. Still
 *    nothing is written to the site.
 * 4. Writes: keeps the installer and .sig in installers/ (the last good
 *    signed installer, for a manual install), copies the installer into the
 *    portfolio's tote/download/ (TOTE_SITE_DIR, default
 *    C:/Users/User/Documents/Portfolio_Lex/tote), publishes
 *    THIRD-PARTY-NOTICES.txt as tote/third-party-notices.txt, writes
 *    tote/latest.json (which every installed Tote reads once a day), updates
 *    the file name, version, size and SHA-256 on tote/index.html, removes
 *    older installers there, and tags this commit v<version>.
 *
 * It stops before committing or deploying: review the portfolio diff, commit,
 * then run `vercel deploy --prod` in the portfolio folder.
 */

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  INSTALLER_NAME,
  cleanTreeProblem,
  decodePublicKey,
  keyIdText,
  mergeConfig,
  newerThanProblem,
  pageFacts,
  passwordProblem,
  sizeLabel,
  trustedFields,
  updatePage,
  verifySignature,
  versionProblems,
  withoutSigningKey,
} from "./release-checks.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SITE = resolve(process.env.TOTE_SITE_DIR ?? "C:/Users/User/Documents/Portfolio_Lex/tote");
const KEY = resolve(process.env.TOTE_UPDATER_KEY ?? join(homedir(), ".tauri", "tote-updater.key"));
const PUBLIC_BASE = "https://www.codewithlex.site/tote";
const TAURI_CLI = join(ROOT, "node_modules", "@tauri-apps", "cli", "tauri.js");
const ARCHIVE = join(ROOT, "installers");

function fail(message) {
  process.stderr.write(`[RELEASE FAIL] ${message}\n`);
  process.exit(1);
}

function failAll(problems, after) {
  for (const problem of problems) process.stderr.write(`[RELEASE FAIL] ${problem}\n`);
  process.stderr.write(`${after}\n`);
  process.exit(1);
}

function git(args, cwd) {
  const run = spawnSync("git", args, { cwd, encoding: "utf8" });
  return { ok: run.status === 0, out: (run.stdout ?? "").trim(), err: (run.stderr ?? "").trim() || (run.error?.message ?? "") };
}

const read = (path) => readFileSync(path, "utf8");

const args = process.argv.slice(2);
const valueAfter = (flag) => {
  const at = args.indexOf(flag);
  if (at < 0) return undefined;
  const value = args[at + 1];
  return value && !value.startsWith("--") ? value : "";
};
const notes = valueAfter("--notes");
const skipBuild = args.includes("--skip-build");
const keyChangeFile = valueAfter("--key-change");
const keyChange = keyChangeFile !== undefined;
const password = process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD;

// 1. Preflight: every check, nothing written. All problems are listed together.
const problems = [];
const note = (problem) => {
  if (problem) problems.push(problem);
};

if (!notes) note('add --notes "What changed, in one plain sentence." (she sees it as the update description)');
else if (/[\u2013\u2014]/.test(notes)) note("the notes contain an en or em dash; use a comma, colon or full stop");

const tauriConfText = read(join(ROOT, "src-tauri", "tauri.conf.json"));
const tauriConf = JSON.parse(tauriConfText);
const version = tauriConf.version;
for (const problem of versionProblems({
  packageJson: read(join(ROOT, "package.json")),
  tauriConf: tauriConfText,
  cargoToml: read(join(ROOT, "src-tauri", "Cargo.toml")),
})) {
  note(problem);
}
const exeName = `Tote_${version}_x64-setup.exe`;
const bundleDir = join(ROOT, "src-tauri", "target", "release", "bundle", "nsis");
const exePath = join(bundleDir, exeName);
const sigPath = `${exePath}.sig`;
const tag = `v${version}`;

// The key installed copies trust: tauri.conf.json's, except in the key change
// release, which is signed with the old key and checked against it.
const configKey = tauriConf.plugins?.updater?.pubkey;
let trustedKey = configKey;
let trustedFrom = "src-tauri/tauri.conf.json";
try {
  decodePublicKey(configKey);
} catch (error) {
  note(`the updater public key in src-tauri/tauri.conf.json cannot be read (${error.message})`);
}
if (keyChange) {
  if (!keyChangeFile) note("--key-change needs the old public key file (the .pub next to the old key)");
  else if (!existsSync(keyChangeFile)) note(`no public key file at ${keyChangeFile}`);
  else {
    trustedKey = read(keyChangeFile);
    trustedFrom = keyChangeFile;
    try {
      const oldId = keyIdText(decodePublicKey(trustedKey).keyId);
      if (oldId === keyIdText(decodePublicKey(configKey).keyId)) {
        note(`--key-change names key ${oldId}, which tauri.conf.json already carries; it is only for the release that moves to a new key`);
      }
    } catch (error) {
      note(`the public key in ${keyChangeFile} cannot be read (${error.message})`);
    }
  }
}

// The site: a page naming one installer that is really there, and a manifest to compare with.
const pagePath = join(SITE, "index.html");
const downloadDir = join(SITE, "download");
const localManifest = join(SITE, "latest.json");
let current;
if (!existsSync(pagePath)) note(`no download page at ${SITE} (set TOTE_SITE_DIR)`);
else if (!existsSync(downloadDir)) note(`no download folder at ${downloadDir}`);
else {
  try {
    const facts = pageFacts(read(pagePath));
    const linked = join(downloadDir, facts.name);
    if (!existsSync(linked)) note(`index.html links ${facts.name}, which is not in ${downloadDir}; check the page by hand`);
    else current = { ...facts, sizeLabel: sizeLabel(statSync(linked).size) };
  } catch (error) {
    note(error.message);
  }
}
if (existsSync(localManifest)) {
  try {
    note(newerThanProblem(version, JSON.parse(read(localManifest)).version, "the portfolio's tote/latest.json"));
  } catch (error) {
    note(`the portfolio's tote/latest.json cannot be read (${error.message})`);
  }
}
try {
  const response = await fetch(`${PUBLIC_BASE}/latest.json`, { headers: { "cache-control": "no-cache" }, signal: AbortSignal.timeout(20_000) });
  if (!response.ok) throw new Error(`the site answered ${response.status}`);
  note(newerThanProblem(version, (await response.json()).version, `${PUBLIC_BASE}/latest.json`));
} catch (error) {
  note(`could not read the published ${PUBLIC_BASE}/latest.json (${error.message}); check the connection and run again`);
}

// Both trees committed, so the tag names what was built and the deploy uploads only the release.
const toteStatus = git(["status", "--porcelain"], ROOT);
note(toteStatus.ok ? cleanTreeProblem(toteStatus.out, "this checkout") : `git status failed in this checkout (${toteStatus.err})`);
if (existsSync(SITE)) {
  const siteStatus = git(["status", "--porcelain"], SITE);
  note(siteStatus.ok ? cleanTreeProblem(siteStatus.out, `the portfolio (${SITE})`) : `the portfolio folder is not a git checkout (${siteStatus.err})`);
}
if (git(["rev-parse", "-q", "--verify", `refs/tags/${tag}`], ROOT).ok) note(`the tag ${tag} already exists, so ${version} was released before; raise the version`);

if (!skipBuild) {
  if (!existsSync(KEY)) note(`no updater key at ${KEY}. Without it no installed Tote will accept the update`);
  note(passwordProblem(password, { keyChange }));
  if (ROOT.includes("'") && !process.env.TAURI_CONFIG) note("this checkout path has an apostrophe; set TAURI_CONFIG as described in README (Building on a path with an apostrophe)");
  if (process.env.TAURI_CONFIG) {
    try {
      JSON.parse(process.env.TAURI_CONFIG);
    } catch (error) {
      note(`TAURI_CONFIG is not valid JSON (${error.message})`);
    }
  }
  if (!existsSync(TAURI_CLI)) note("the Tauri CLI is not in node_modules; run npm install first");
}

if (problems.length) failAll(problems, "Nothing was built or written.");
if (keyChange && !skipBuild && !password) {
  process.stdout.write("Signing with the old updater key, which has no passphrase. Once her laptop runs this version, delete the old key (README, The updater key).\n");
}

// 2. Build without the key, then sign in a step of its own.
if (!skipBuild) {
  // License notices for everything compiled in; bundled next to tote.exe.
  const notices = spawnSync(process.execPath, [join(ROOT, "scripts", "third-party-notices.mjs")], { cwd: ROOT, stdio: "inherit" });
  if (notices.status !== 0) fail("could not write THIRD-PARTY-NOTICES.txt");

  // createUpdaterArtifacts makes `tauri build` sign; it is turned off for this
  // build only, merged over TAURI_CONFIG so the icon override still applies.
  const buildEnv = withoutSigningKey(process.env);
  const overrides = mergeConfig(process.env.TAURI_CONFIG ? JSON.parse(process.env.TAURI_CONFIG) : {}, { bundle: { createUpdaterArtifacts: false } });
  const configFile = join(tmpdir(), `tote-release-config-${process.pid}.json`);
  writeFileSync(configFile, JSON.stringify(overrides));
  process.stdout.write(`Building Tote ${version}...\n`);
  const build = spawnSync(process.execPath, [TAURI_CLI, "build", "--config", configFile], { cwd: ROOT, stdio: "inherit", env: buildEnv });
  rmSync(configFile, { force: true });
  if (build.status !== 0) fail(`tauri build exited with ${build.status ?? build.error?.message}`);
  if (!existsSync(exePath)) fail(`the build made no installer at ${exePath}`);

  // A .sig left from an earlier build must never pass for this installer.
  rmSync(sigPath, { force: true });
  process.stdout.write(`Signing ${exeName}...\n`);
  const signArgs = [TAURI_CLI, "signer", "sign", "--app-version", version, "--private-key-path", KEY];
  // The old key of a key change release has no passphrase; the empty one is passed as such.
  if (keyChange && !password) signArgs.push("--password", "");
  signArgs.push(exePath);
  const signEnv = password ? { ...buildEnv, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password } : buildEnv;
  const sign = spawnSync(process.execPath, signArgs, { cwd: ROOT, stdio: "inherit", env: signEnv });
  if (sign.status !== 0) fail(`signing exited with ${sign.status ?? sign.error?.message}; check the passphrase, then run the release again`);
}

// 3. What will be published, checked the way the updater checks it. Still nothing written.
const after = [];
if (!existsSync(exePath)) after.push(`no installer at ${exePath}; build first`);
else if (!existsSync(sigPath)) after.push(`no signature at ${sigPath}; sign the installer first`);
let bytes;
let next;
if (!after.length) {
  bytes = readFileSync(exePath);
  try {
    const signed = verifySignature(bytes, read(sigPath), trustedKey);
    const fields = trustedFields(signed.trustedComment);
    if (fields.version !== version) after.push(`the signature names version ${fields.version ?? "(none)"}, not ${version}; sign with --app-version ${version}`);
    if (fields.file !== undefined && basename(fields.file) !== exeName) after.push(`the signature was made for ${fields.file}, not ${exeName}`);
  } catch (error) {
    after.push(`the signature does not check out against the key in ${trustedFrom}: ${error.message}. Installed copies would refuse this update; build and sign again`);
  }
  next = { name: exeName, version, sizeLabel: sizeLabel(bytes.length), sha256: createHash("sha256").update(bytes).digest("hex") };
}
const noticesPath = join(ROOT, "THIRD-PARTY-NOTICES.txt");
if (!existsSync(noticesPath)) after.push("no THIRD-PARTY-NOTICES.txt; run node scripts/third-party-notices.mjs");
const builtStatus = git(["status", "--porcelain"], ROOT);
if (!builtStatus.ok) after.push(`git status failed in this checkout (${builtStatus.err})`);
else {
  const changed = cleanTreeProblem(builtStatus.out, "this checkout after the build");
  if (changed) after.push(`${changed} (the build changed tracked files, so the tag would not name what was built)`);
}
let page;
if (next) {
  try {
    page = updatePage(read(pagePath), current, next);
  } catch (error) {
    after.push(error.message);
  }
}
if (after.length) failAll(after, "Nothing was written to the site.");

// 4. Writes. Nothing is live until the portfolio is deployed; a failure names
// what was done so the portfolio can be put back first.
const done = [];
function step(label, action) {
  try {
    action();
    done.push(label);
  } catch (error) {
    fail(`${label} failed (${error.message}). Done before it: ${done.join(", ") || "nothing"}. Run git status in the portfolio and undo its changes before trying again.`);
  }
}
step("keeping the installer in installers/", () => {
  mkdirSync(ARCHIVE, { recursive: true });
  copyFileSync(exePath, join(ARCHIVE, exeName));
  copyFileSync(sigPath, join(ARCHIVE, `${exeName}.sig`));
});
step("copying the installer", () => copyFileSync(exePath, join(downloadDir, exeName)));
step("publishing the notices", () => copyFileSync(noticesPath, join(SITE, "third-party-notices.txt")));
step("writing latest.json", () => {
  const manifest = {
    version,
    notes,
    pub_date: new Date().toISOString(),
    platforms: {
      "windows-x86_64": {
        signature: read(sigPath).trim(),
        url: `${PUBLIC_BASE}/download/${exeName}`,
      },
    },
  };
  writeFileSync(localManifest, `${JSON.stringify(manifest, null, 2)}\n`);
});
step("updating the page", () => writeFileSync(pagePath, page));
step("removing older installers", () => {
  for (const old of readdirSync(downloadDir).filter((f) => new RegExp(`^${INSTALLER_NAME.source}$`).test(f) && f !== exeName)) {
    rmSync(join(downloadDir, old));
    process.stdout.write(`removed ${old}\n`);
  }
});
const tagged = git(["tag", "-a", tag, "-m", `Tote ${version}`], ROOT);
if (!tagged.ok) fail(`the site files are written, but tagging failed (${tagged.err}); tag by hand: git tag -a ${tag} -m "Tote ${version}"`);

process.stdout.write(
  [
    "",
    `[RELEASE READY] Tote ${version}`,
    `  installer  ${join(downloadDir, exeName)} (${next.sizeLabel}, ${bytes.length} bytes)`,
    `  SHA-256    ${next.sha256}`,
    `  manifest   ${localManifest}`,
    `  signature  checked against key ${keyIdText(decodePublicKey(trustedKey).keyId)} (${trustedFrom})`,
    `  tag        ${tag} on this commit`,
    "Next: review the portfolio diff, commit, then run `vercel deploy --prod` in the portfolio folder.",
    `Then push the tag with the branch: git push origin ${tag}`,
    "Installed copies offer the update within a day of their last check.",
    "",
  ].join("\n"),
);
