/**
 * The checks scripts/release.mjs runs before it writes anything, kept apart
 * so they can be tested without building or publishing
 * (scripts/release-checks.test.mjs).
 *
 * The signature check is the one the updater runs on her laptop
 * (tauri-plugin-updater through minisign-verify): the .sig must come from the
 * key installed copies trust, its Ed25519 signature must cover the
 * installer's bytes (BLAKE2b-512 prehashed for "ED", the bytes themselves for
 * the legacy "Ed"), and its global signature must cover the trusted comment,
 * which is what makes the "version:" field in it trustworthy.
 */

import { createHash, createPublicKey, verify } from "node:crypto";

const UNTRUSTED = "untrusted comment:";
const TRUSTED = "trusted comment: ";

/** Tauri keeps minisign texts base64 encoded (tauri.conf.json pubkey, .sig files); accepts either form. */
export function minisignText(value) {
  const text = String(value ?? "").trim();
  return text.startsWith(UNTRUSTED) ? text : Buffer.from(text, "base64").toString("utf8");
}

function minisignLines(value, what) {
  const text = minisignText(value);
  if (!text.startsWith(UNTRUSTED)) throw new Error(`${what} is not a minisign text`);
  return text.split(/\r?\n/);
}

function exactBase64(line, length, what) {
  const trimmed = (line ?? "").trim();
  const bytes = Buffer.from(trimmed, "base64");
  if (!trimmed || bytes.length !== length || bytes.toString("base64") !== trimmed) {
    throw new Error(`${what} is damaged`);
  }
  return bytes;
}

/** minisign shows a key id as the hex of its eight bytes read little-endian ("3EC8BBEA531C72C7"). */
export function keyIdText(keyId) {
  return Buffer.from(keyId).reverse().toString("hex").toUpperCase();
}

export function decodePublicKey(value) {
  const bin = exactBase64(minisignLines(value, "the public key")[1], 42, "the public key");
  const algorithm = bin.subarray(0, 2).toString("latin1");
  if (algorithm !== "Ed" && algorithm !== "ED") throw new Error("the public key is not a minisign Ed25519 key");
  return { keyId: bin.subarray(2, 10), key: bin.subarray(10, 42) };
}

export function decodeSignature(value) {
  const [, signatureLine, trustedLine, globalLine] = minisignLines(value, "the signature");
  const bin = exactBase64(signatureLine, 74, "the signature");
  const algorithm = bin.subarray(0, 2).toString("latin1");
  if (algorithm !== "Ed" && algorithm !== "ED") throw new Error("the signature is not a minisign Ed25519 signature");
  if (!trustedLine || !trustedLine.startsWith(TRUSTED)) throw new Error("the signature has no trusted comment");
  return {
    prehashed: algorithm === "ED",
    keyId: bin.subarray(2, 10),
    signature: bin.subarray(10, 74),
    trustedComment: trustedLine.slice(TRUSTED.length),
    globalSignature: exactBase64(globalLine, 64, "the signature's global signature"),
  };
}

function ed25519(raw) {
  return createPublicKey({ key: { kty: "OKP", crv: "Ed25519", x: Buffer.from(raw).toString("base64url") }, format: "jwk" });
}

/**
 * Verifies `signatureValue` (the .sig file's text) over `data` with
 * `publicKeyValue` (the tauri.conf.json pubkey or a .pub file) and returns
 * the trusted comment; throws with the reason in plain words otherwise.
 */
export function verifySignature(data, signatureValue, publicKeyValue) {
  const publicKey = decodePublicKey(publicKeyValue);
  const signature = decodeSignature(signatureValue);
  if (!publicKey.keyId.equals(signature.keyId)) {
    throw new Error(`it was made with key ${keyIdText(signature.keyId)}, not with key ${keyIdText(publicKey.keyId)} that installed copies trust`);
  }
  const key = ed25519(publicKey.key);
  const message = signature.prehashed ? createHash("blake2b512").update(data).digest() : data;
  if (!verify(null, message, key, signature.signature)) throw new Error("it does not match the installer's bytes");
  const global = Buffer.concat([signature.signature, Buffer.from(signature.trustedComment, "utf8")]);
  if (!verify(null, global, key, signature.globalSignature)) throw new Error("its trusted comment was changed after signing");
  return { trustedComment: signature.trustedComment, keyId: keyIdText(signature.keyId) };
}

/** The tab separated `key:value` fields Tauri writes in a trusted comment; the first of each name counts, as in the updater. */
export function trustedFields(comment) {
  const fields = {};
  for (const part of String(comment).split("\t")) {
    const at = part.indexOf(":");
    if (at > 0 && !(part.slice(0, at) in fields)) fields[part.slice(0, at)] = part.slice(at + 1);
  }
  return fields;
}

const SEMVER = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/;

export function parseVersion(text) {
  const match = SEMVER.exec(String(text ?? "").trim());
  return match ? { core: match.slice(1, 4).map(Number), pre: match[4] ? match[4].split(".") : [] } : null;
}

/** Semver precedence: -1, 0 or 1. A pre-release sorts before its release. */
export function compareVersions(a, b) {
  const x = parseVersion(a);
  const y = parseVersion(b);
  if (!x || !y) throw new Error(`"${x ? b : a}" is not a version`);
  for (let i = 0; i < 3; i += 1) {
    if (x.core[i] !== y.core[i]) return x.core[i] < y.core[i] ? -1 : 1;
  }
  if (!x.pre.length || !y.pre.length) return Math.sign(y.pre.length - x.pre.length);
  for (let i = 0; i < Math.max(x.pre.length, y.pre.length); i += 1) {
    const p = x.pre[i];
    const q = y.pre[i];
    if (p === undefined || q === undefined) return p === undefined ? -1 : 1;
    const pNumber = /^\d+$/.test(p);
    const qNumber = /^\d+$/.test(q);
    if (pNumber && qNumber && Number(p) !== Number(q)) return Number(p) < Number(q) ? -1 : 1;
    if (pNumber !== qNumber) return pNumber ? -1 : 1;
    if (!pNumber && p !== q) return p < q ? -1 : 1;
  }
  return 0;
}

/** The `version` of the `[package]` table in a Cargo.toml, or null. */
export function cargoPackageVersion(toml) {
  const table = /^\[package\][^\S\n]*\r?$([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(String(toml));
  const version = table && /^\s*version\s*=\s*"([^"]+)"/m.exec(table[1]);
  return version ? version[1] : null;
}

/** Problems with the three version fields: each must be a version and all three must agree. */
export function versionProblems({ packageJson, tauriConf, cargoToml }) {
  const found = {
    "package.json": JSON.parse(packageJson).version,
    "src-tauri/tauri.conf.json": JSON.parse(tauriConf).version,
    "src-tauri/Cargo.toml": cargoPackageVersion(cargoToml),
  };
  const problems = Object.entries(found)
    .filter(([, version]) => !parseVersion(version))
    .map(([file, version]) => `${file} has no usable version (found ${JSON.stringify(version ?? null)})`);
  if (!problems.length && new Set(Object.values(found)).size > 1) {
    const listed = Object.entries(found).map(([file, version]) => `${file} ${version}`).join(", ");
    problems.push(`the version is not the same everywhere (${listed}); raise all three together`);
  }
  return problems;
}

/** A problem when `version` is not higher than the one `where` already offers. */
export function newerThanProblem(version, published, where) {
  if (published === undefined || published === null) return null;
  if (!parseVersion(published)) return `${where} names "${published}", which is not a version; check it by hand`;
  return compareVersions(version, published) > 0
    ? null
    : `version ${version} is not higher than ${published} in ${where}; installed copies only take a higher version, so raise it first`;
}

/**
 * The updater key must carry a passphrase (audit T-05): an empty one means
 * the key file alone is enough to sign an update. The one exception is the
 * key change release, signed with the old key, which has none.
 */
export function passwordProblem(password, { keyChange = false } = {}) {
  if (typeof password === "string" && password.trim() !== "") return null;
  if (keyChange) return null;
  return "TAURI_SIGNING_PRIVATE_KEY_PASSWORD is empty; the updater key must have a passphrase (README, The updater key). Set it in this terminal only, then run again";
}

/** A problem when `git status --porcelain` printed anything. */
export function cleanTreeProblem(porcelain, label) {
  const changed = String(porcelain).split(/\r?\n/).filter((line) => line.trim() !== "");
  if (!changed.length) return null;
  const names = changed.slice(0, 3).map((line) => line.slice(3)).join(", ");
  return `${label} has ${changed.length} uncommitted change(s) (${names}${changed.length > 3 ? ", ..." : ""}); commit or remove them first`;
}

export const INSTALLER_NAME = /Tote_\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?_x64-setup\.exe/g;
const CHECKSUM = /\b[0-9a-f]{64}\b/g;

/** What the download page names today: one installer (and so its version) and one SHA-256, or it throws. */
export function pageFacts(page) {
  const names = [...new Set(String(page).match(INSTALLER_NAME) ?? [])];
  const sums = [...new Set(String(page).match(CHECKSUM) ?? [])];
  if (names.length !== 1) throw new Error(`index.html names ${names.length} different installers (${names.join(", ") || "none"}); it must name exactly one, so check the page by hand`);
  if (sums.length !== 1) throw new Error(`index.html shows ${sums.length} different SHA-256 values; it must show exactly one, so check the page by hand`);
  return { name: names[0], version: names[0].slice("Tote_".length, -"_x64-setup.exe".length), sha256: sums[0] };
}

export function sizeLabel(bytes) {
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function token(text) {
  const escaped = text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(?<![\\w.])${escaped}(?![\\w]|\\.\\d)`, "g");
}

/**
 * The download page for the new installer: only the four values that name
 * the installer it links today change (its file name, "version X", its size
 * label and its SHA-256), never other text that happens to look like them.
 * Throws when one of the four is missing, so nothing is written on a page
 * that needs a look by hand.
 */
export function updatePage(page, current, next) {
  const swaps = [
    [current.name, next.name, "installer name"],
    [`version ${current.version}`, `version ${next.version}`, "version"],
    [current.sizeLabel, next.sizeLabel, "size"],
    [current.sha256, next.sha256, "SHA-256"],
  ];
  let out = String(page);
  for (const [from, to, label] of swaps) {
    const pattern = token(from);
    if (!pattern.test(out)) throw new Error(`index.html has no ${label} "${from}" to update; check the page by hand`);
    out = out.replace(token(from), () => to);
  }
  return out;
}

/** `overrides` merged into `base`: objects merge key by key, anything else replaces. */
export function mergeConfig(base, overrides) {
  const isObject = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
  if (!isObject(base) || !isObject(overrides)) return overrides;
  const out = { ...base };
  for (const [key, value] of Object.entries(overrides)) out[key] = key in out ? mergeConfig(out[key], value) : value;
  return out;
}

/** The environment without the updater key, for every step except signing. */
export function withoutSigningKey(env) {
  const out = { ...env };
  for (const name of Object.keys(out)) {
    if (/^TAURI_SIGNING_PRIVATE_KEY/i.test(name)) delete out[name];
  }
  return out;
}
