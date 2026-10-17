/**
 * Tests for the release preflight (scripts/release-checks.mjs) and the
 * notices generator's license rule. Nothing is built or published.
 *
 *   node --test scripts/release-checks.test.mjs
 */

import assert from "node:assert/strict";
import { createHash, generateKeyPairSync, sign } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  cargoPackageVersion,
  cleanTreeProblem,
  compareVersions,
  keyIdText,
  mergeConfig,
  newerThanProblem,
  pageFacts,
  passwordProblem,
  trustedFields,
  updatePage,
  verifySignature,
  versionProblems,
  withoutSigningKey,
} from "./release-checks.mjs";
import { packagesWithoutText, withVendoredTexts } from "./third-party-notices.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const b64 = (text) => Buffer.from(text, "utf8").toString("base64");

// A key and signatures in Tauri's form: base64 of minisign texts, the
// installer prehashed with BLAKE2b-512 ("ED"), the version in the trusted comment.
function tauriKey() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const raw = Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url");
  const keyId = Buffer.from("0123456789abcdef", "hex");
  const pub = b64(`untrusted comment: minisign public key: ${keyIdText(keyId)}\n${Buffer.concat([Buffer.from("Ed"), keyId, raw]).toString("base64")}\n`);
  const signFile = (data, trusted, id = keyId) => {
    const signature = sign(null, createHash("blake2b512").update(data).digest(), privateKey);
    const global = sign(null, Buffer.concat([signature, Buffer.from(trusted)]), privateKey);
    const box = Buffer.concat([Buffer.from("ED"), id, signature]).toString("base64");
    return b64(`untrusted comment: signature from tauri secret key\n${box}\ntrusted comment: ${trusted}\n${global.toString("base64")}\n`);
  };
  return { pub, signFile };
}

test("an empty updater passphrase is refused (audit T-05)", () => {
  for (const empty of [undefined, "", "   "]) assert.match(passwordProblem(empty), /TAURI_SIGNING_PRIVATE_KEY_PASSWORD is empty/);
  assert.equal(passwordProblem("a long passphrase"), null);
  assert.equal(passwordProblem("", { keyChange: true }), null, "the key change release is signed with the old key, which has none");
});

test("the signing step never sees the key in the build's environment", () => {
  const env = withoutSigningKey({ PATH: "p", TAURI_SIGNING_PRIVATE_KEY: "k", TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "s", TAURI_SIGNING_PRIVATE_KEY_PATH: "f", TAURI_CONFIG: "{}" });
  assert.deepEqual(env, { PATH: "p", TAURI_CONFIG: "{}" });
});

test("minisign's own test vectors verify, legacy and prehashed", () => {
  const pub = "untrusted comment: minisign public key E7620F1842B4E81F\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
  const legacy = [
    "untrusted comment: signature from minisign secret key",
    "RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=",
    "trusted comment: timestamp:1555779966\tfile:test",
    "QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==",
  ].join("\n");
  const prehashed = [
    "untrusted comment: signature from minisign secret key",
    "RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=",
    "trusted comment: timestamp:1556193335\tfile:test",
    "y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==",
  ].join("\n");
  assert.equal(verifySignature(Buffer.from("test"), legacy, pub).trustedComment, "timestamp:1555779966\tfile:test");
  assert.equal(verifySignature(Buffer.from("test"), b64(prehashed), b64(pub)).keyId, "E7620F1842B4E81F");
  assert.throws(() => verifySignature(Buffer.from("Test"), legacy, pub), /does not match the installer's bytes/);
});

test("a Tauri signature is checked for bytes, key, trusted comment and version (audit T-33)", () => {
  const { pub, signFile } = tauriKey();
  const installer = Buffer.from("the installer bytes");
  const trusted = "timestamp:1791961796\tfile:Tote_0.4.0_x64-setup.exe\tversion:0.4.0";
  const good = signFile(installer, trusted);
  const fields = trustedFields(verifySignature(installer, good, pub).trustedComment);
  assert.deepEqual(fields, { timestamp: "1791961796", file: "Tote_0.4.0_x64-setup.exe", version: "0.4.0" });

  assert.throws(() => verifySignature(Buffer.from("a rebuilt installer"), good, pub), /does not match the installer's bytes/, "a .sig left beside a rebuilt installer, as 0.2.1 was");
  const lines = Buffer.from(good, "base64").toString().split("\n");
  lines[2] = lines[2].replace("version:0.4.0", "version:9.9.9");
  assert.throws(() => verifySignature(installer, b64(lines.join("\n")), pub), /trusted comment was changed/);
  assert.throws(() => verifySignature(installer, signFile(installer, trusted, Buffer.alloc(8, 7)), pub), /made with key 0707070707070707, not with key EFCDAB8967452301/);
  assert.throws(() => verifySignature(installer, good, tauriKey().pub), /does not match/);
  assert.throws(() => verifySignature(installer, "bm90IGEgc2lnbmF0dXJl", pub), /not a minisign text/);
});

test("the published 0.3.0 installer verifies and the stale 0.2.1 pair does not", (t) => {
  const bundle = join(ROOT, "src-tauri", "target", "release", "bundle", "nsis");
  const pair = (v) => join(bundle, `Tote_${v}_x64-setup.exe`);
  if (!existsSync(`${pair("0.3.0")}.sig`) || !existsSync(`${pair("0.2.1")}.sig`)) {
    t.skip("the 0.2.1 and 0.3.0 installers are only on Lex's machine");
    return;
  }
  const pub = JSON.parse(readFileSync(join(ROOT, "src-tauri", "tauri.conf.json"), "utf8")).plugins.updater.pubkey;
  const signed = verifySignature(readFileSync(pair("0.3.0")), readFileSync(`${pair("0.3.0")}.sig`, "utf8"), pub);
  assert.equal(trustedFields(signed.trustedComment).version, "0.3.0");
  assert.equal(signed.keyId, "3EC8BBEA531C72C7");
  assert.throws(() => verifySignature(readFileSync(pair("0.2.1")), readFileSync(`${pair("0.2.1")}.sig`, "utf8"), pub), /does not match/);
});

test("the three version fields must agree and be versions", () => {
  const files = (pkg, conf, cargo) => ({
    packageJson: JSON.stringify({ version: pkg }),
    tauriConf: JSON.stringify({ version: conf }),
    cargoToml: `[package]\nname = "tote"\nversion = "${cargo}"\n\n[dependencies]\nserde = { version = "1" }\n`,
  });
  assert.deepEqual(versionProblems(files("0.4.0", "0.4.0", "0.4.0")), []);
  assert.match(versionProblems(files("0.4.0", "0.4.0", "0.3.0")).join(), /not the same everywhere.*Cargo.toml 0.3.0/);
  assert.match(versionProblems(files("0.4", "0.4.0", "0.4.0")).join(), /package.json has no usable version/);
  assert.equal(cargoPackageVersion(readFileSync(join(ROOT, "src-tauri", "Cargo.toml"), "utf8")), JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8")).version);
  assert.equal(cargoPackageVersion('[dependencies]\nversion = "9.9.9"\n'), null, "only the [package] table counts");
});

test("a release must be higher than what is published", () => {
  assert.equal(compareVersions("0.10.0", "0.9.9"), 1);
  assert.equal(compareVersions("0.3.0", "v0.3.0"), 0);
  assert.equal(compareVersions("1.0.0-beta.2", "1.0.0-beta.11"), -1);
  assert.equal(compareVersions("1.0.0-beta", "1.0.0"), -1);
  assert.equal(compareVersions("1.0.0-alpha.1", "1.0.0-alpha"), 1);
  assert.equal(newerThanProblem("0.4.0", "0.3.0", "latest.json"), null);
  assert.match(newerThanProblem("0.3.0", "0.3.0", "latest.json"), /not higher than 0.3.0/);
  assert.match(newerThanProblem("0.2.9", "0.3.0", "latest.json"), /not higher/);
  assert.equal(newerThanProblem("0.4.0", undefined, "latest.json"), null);
});

test("both trees must be committed", () => {
  assert.equal(cleanTreeProblem("", "this checkout"), null);
  assert.match(cleanTreeProblem(" M README.md\n?? notes.txt\n", "this checkout"), /2 uncommitted change\(s\) \(README.md, notes.txt\)/);
});

test("the page changes only the values that name the linked installer", () => {
  const page = [
    '<a href="/tote/download/Tote_0.3.0_x64-setup.exe">Download</a>',
    "<p>Free. 5.1 MB, version 0.3.0.</p>",
    "<p>Tote_0.3.0_x64-setup.exe, 5.1 MB</p>",
    "<code>b2fdaffa00fe1de81bcb8554acffbac6f4e792e2ddb67bfe8246994f8d2308d5</code>",
    "<p>Needs about 15.1 MB of space and Windows version 11.</p>",
  ].join("\n");
  const facts = pageFacts(page);
  assert.deepEqual(facts, { name: "Tote_0.3.0_x64-setup.exe", version: "0.3.0", sha256: "b2fdaffa00fe1de81bcb8554acffbac6f4e792e2ddb67bfe8246994f8d2308d5" });
  const sha = "a".repeat(64);
  const out = updatePage(page, { ...facts, sizeLabel: "5.1 MB" }, { name: "Tote_0.4.0_x64-setup.exe", version: "0.4.0", sizeLabel: "5.3 MB", sha256: sha });
  assert.equal(out.match(/Tote_0\.4\.0_x64-setup\.exe/g).length, 2);
  assert.match(out, /5\.3 MB, version 0\.4\.0\./);
  assert.match(out, new RegExp(`<code>${sha}</code>`));
  assert.match(out, /about 15\.1 MB of space and Windows version 11/, "other sizes and versions on the page stay");
  assert.throws(() => updatePage(page.replace(/version 0\.3\.0/, "v0.3.0"), { ...facts, sizeLabel: "5.1 MB" }, { name: "x", version: "0.4.0", sizeLabel: "5.3 MB", sha256: sha }), /no version "version 0.3.0"/);
  assert.throws(() => pageFacts(`${page}\nTote_0.2.0_x64-setup.exe`), /names 2 different installers/);
});

test("the build's config override keeps the icon override", () => {
  const merged = mergeConfig({ bundle: { icon: ["a.png"] } }, { bundle: { createUpdaterArtifacts: false } });
  assert.deepEqual(merged, { bundle: { icon: ["a.png"], createUpdaterArtifacts: false } });
});

test("the notices generator names every shipped package without a license text (audit T-39)", () => {
  const packages = [
    { name: "has-own", version: "1.0.0", texts: ["MIT License"] },
    { name: "pdf-extract", version: "0.12.1", texts: [] },
    { name: "nowhere", version: "2.0.0", texts: [] },
  ];
  assert.deepEqual(packagesWithoutText(packages).map((p) => p.name), ["pdf-extract", "nowhere"]);

  const filled = withVendoredTexts(packages, join(ROOT, "licenses"));
  assert.deepEqual(filled[0], packages[0], "a package's own file wins");
  assert.match(filled[1].texts.join(), /Permission is hereby granted/, "pdf-extract takes its text from licenses/");
  assert.deepEqual(packagesWithoutText(filled).map((p) => p.name), ["nowhere"], "a package in neither place still fails the generator");
  for (const name of ["adobe-cmap-parser", "alloc-stdlib", "clipboard-win", "defmt-parser", "selectors", "type1-encoding-parser", "webview2-com", "webview2-com-macros", "webview2-com-sys", "webview2-loader"]) {
    const [entry] = withVendoredTexts([{ name, version: "0", texts: [] }], join(ROOT, "licenses"));
    assert.ok(entry.texts.length > 0, `licenses/${name}/ holds a text`);
  }
});
