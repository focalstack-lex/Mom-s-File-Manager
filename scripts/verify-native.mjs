#!/usr/bin/env node
/**
 * Tote native check: the real app, real files, driven through its own window.
 *
 * Builds a throwaway folder set, launches the release tote.exe in test mode
 * (TOTE_ROOTS and TOTE_DATA_DIR, so nothing real is touched and autostart is
 * left alone) with WebView2 remote debugging on, then drives the actual
 * window over CDP: drops downloads with Zone.Identifier streams and waits for
 * them to be filed, searches the real index, moves, renames, pins, recycles
 * and undoes through the real commands. Evidence lands in
 * reports/native-check/<stamp>/.
 *
 * Usage: node scripts/verify-native.mjs   (needs src-tauri/target/release/tote.exe)
 */
import { spawn, execFileSync } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
// The fresh build when there is one; otherwise the copy kept in installers/
// (cargo clean removes target/ to save space).
const BUILT_EXE = join(ROOT, "src-tauri", "target", "release", "tote.exe");
const EXE = existsSync(BUILT_EXE) ? BUILT_EXE : join(ROOT, "installers", "tote-release.exe");
const LAB = resolve(process.env.LOCALAPPDATA ?? ROOT, "tote-native-check");
// A file whose full path passes the 260 characters the Recycle Bin accepts (audit T-01).
const DEEP_DIR = join(LAB, "Documents", "Forms", "A".repeat(120), "B".repeat(120));
const DEEP_FILE = join(DEEP_DIR, "Deep Quiz.docx");
const PORT = 9333;
const STEP_TIMEOUT_MS = 10_000;
const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
const EVIDENCE_DIR = join(ROOT, "reports", "native-check", stamp);
const results = [];
const pageErrors = [];

function record(step, passed, detail) {
  results.push({ step, passed, detail });
  process.stdout.write(`[${passed ? "PASS" : "FAIL"}] ${step}${detail ? ` :: ${detail}` : ""}\n`);
}
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

async function until(check, timeout = STEP_TIMEOUT_MS) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await check()) return true;
    await sleep(500);
  }
  return false;
}

function seedLab() {
  rmSync(LAB, { recursive: true, force: true });
  for (const d of ["Desktop", "Documents\\Forms", "Downloads", "data"]) mkdirSync(join(LAB, d), { recursive: true });
  writeFileSync(join(LAB, "Desktop", "Lesson Plan.docx"), "x");
  writeFileSync(join(LAB, "Desktop", "Lesson Plan (2).docx"), "xy");
  writeFileSync(join(LAB, "Desktop", "notes.txt"), "Santos Reyes attendance Grade 7");
  // For Organize: names that carry a grade and a subject, and a shortcut that must stay.
  writeFileSync(join(LAB, "Desktop", "Grade 7 Science Quiz.docx"), "quiz");
  writeFileSync(join(LAB, "Desktop", "AP 8 Reviewer.docx"), "reviewer");
  writeFileSync(join(LAB, "Desktop", "Zoom.lnk"), "shortcut");
  writeFileSync(join(LAB, "Documents", "Forms", "SF9 Template.docx"), "master");
  mkdirSync(DEEP_DIR, { recursive: true });
  writeFileSync(DEEP_FILE, "deep");
  // A code project in Documents: its build output must stay out of the index.
  mkdirSync(join(LAB, "Documents", "app", "target", "release"), { recursive: true });
  writeFileSync(join(LAB, "Documents", "app", "Cargo.toml"), "[package]");
  writeFileSync(join(LAB, "Documents", "app", "target", "release", "zebrajunk.lib"), "x");
  mkdirSync(join(LAB, "Desktop", "Saved Here"), { recursive: true });
  const old = join(LAB, "Downloads", "old attendance.pdf");
  writeFileSync(old, "old");
  const past = new Date(Date.now() - 90 * 86_400_000);
  utimesSync(old, past, past);
  // Files she keeps in Downloads on purpose, there before Tote starts (audit T-07).
  writeFileSync(join(LAB, "Downloads", "Old Lesson.docx"), "a lesson she keeps in Downloads");
  writeFileSync(join(LAB, "Downloads", "Existing Form.pdf"), "%PDF-1.4 a form she keeps in Downloads");
}

/** Writes the Zone.Identifier stream a browser leaves on a finished download. */
function stampDownload(path, hostUrl, referrer) {
  const zone = `[ZoneTransfer]\r\nZoneId=3\r\n${referrer ? `ReferrerUrl=${referrer}\r\n` : ""}HostUrl=${hostUrl}\r\n`;
  execFileSync("powershell", ["-NoProfile", "-Command", `Set-Content -LiteralPath '${path.replace(/'/g, "''")}' -Stream Zone.Identifier -Value '${zone.replace(/'/g, "''")}'`]);
}

/** Writes a file plus the Zone.Identifier stream a browser would leave. */
function fakeDownload(name, body, hostUrl, referrer, folder = join(LAB, "Downloads")) {
  const path = join(folder, name);
  writeFileSync(path, body);
  stampDownload(path, hostUrl, referrer);
  return path;
}

const filed = (kind, name) => existsSync(join(LAB, "Documents", "School Files", kind, new Date().toISOString().slice(0, 7), name));

/** Starts the release app in test mode and attaches to its window over CDP; page is null when it never answers. */
async function launch(env) {
  const app = spawn(EXE, [], { env, stdio: "ignore", detached: false });
  let browser = null;
  const attached = await until(async () => {
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${PORT}`);
      return true;
    } catch {
      return false;
    }
  }, 30_000);
  if (!attached) return { app, browser: null, page: null };
  const context = browser.contexts()[0];
  const page = context.pages().find((p) => p.url().startsWith("tauri://") || p.url().includes("localhost")) ?? context.pages()[0];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.waitForSelector('[data-testid="find-view"]', { timeout: STEP_TIMEOUT_MS });
  return { app, browser, page };
}

/** Drops the CDP link and ends the app, then waits until its debugging port is closed. */
async function stop(session) {
  if (!session) return;
  if (session.browser) await session.browser.close().catch(() => undefined);
  spawn("taskkill", ["/pid", String(session.app.pid), "/T", "/F"], { stdio: "ignore" });
  await until(async () => {
    try {
      await fetch(`http://127.0.0.1:${PORT}/json/version`, { signal: AbortSignal.timeout(500) });
      return false;
    } catch {
      return true;
    }
  }, 15_000);
  await sleep(1000);
}

async function main() {
  if (!existsSync(EXE)) {
    process.stderr.write(`[NATIVE CHECK ERROR] ${EXE} not found. Run npm run build:release first.\n`);
    process.exit(1);
  }
  mkdirSync(EVIDENCE_DIR, { recursive: true });
  seedLab();
  const env = {
    ...process.env,
    TOTE_ROOTS: `${join(LAB, "Desktop")};${join(LAB, "Documents")};${join(LAB, "Downloads")}`,
    TOTE_DATA_DIR: join(LAB, "data"),
    // Only honoured together with TOTE_ROOTS (src-tauri/src/lib.rs), so a normal start never opens a port.
    TOTE_DEBUG_PORT: String(PORT),
    // Opt-in (TOTE_CHECK_UPDATE=1): the app treats the published release as
    // new, downloads it and verifies its signature, and stops before installing.
    ...(process.env.TOTE_CHECK_UPDATE ? { TOTE_UPDATE_VERIFY: "1" } : {}),
  };
  let session = null;
  let exitCode = 0;
  try {
    session = await launch(env);
    const attached = session.page !== null;
    record("The release app starts in test mode and exposes its window over CDP", attached && !session.app.exitCode, attached ? "" : "no CDP endpoint");
    if (!attached) throw new Error("could not attach");
    // The window changes when the app is restarted (steps 11 and 12).
    let page = session.page;
    const capture = (name) => page.screenshot({ path: join(EVIDENCE_DIR, `${name}.png`) });

    // 1. First run shows the walkthrough; Escape skips it and it stays skipped (real settings table).
    const tour = await page.locator('[data-testid="tour"]').isVisible();
    await page.keyboard.press("Escape");
    const tourGone = await until(async () => !(await page.locator('[data-testid="tour"]').isVisible()));
    await capture("01-find");
    record("First run shows the walkthrough; Escape skips it", tour && tourGone, JSON.stringify({ tour, tourGone }));

    // 2. The real index: a student's name finds notes.txt by its contents.
    await page.locator('[data-testid="search-input"]').fill("santos");
    const hit = await until(async () => (await page.locator('[data-testid="results"] .row-name-text').first().textContent().catch(() => "")) === "notes.txt");
    const marked = await page.locator('[data-testid="results"] .row-snippet mark').first().textContent().catch(() => null);
    await capture("02-search");
    record("Real index: 'santos' finds notes.txt by its contents with a marked snippet", hit && marked?.toLowerCase() === "santos", JSON.stringify({ hit, marked }));
    // 2b. Build output beside a project file is not indexed.
    await page.locator('[data-testid="search-input"]').fill("zebrajunk");
    await sleep(1500);
    const junkHits = await page.locator('[data-testid="results"] [data-testid="file-row"]').count();
    record("Build output beside Cargo.toml stays out of search", junkHits === 0, `hits=${junkHits}`);
    await page.locator('[data-testid="search-input"]').fill("");

    // 2c. A browser that saved to another folder: the stamped file joins Recent downloads and stays put.
    const elsewhere = fakeDownload("Me at the zoo.mp3", "ID3 fake", "https://files.example.org/zoo.mp3", "https://www.example.org/", join(LAB, "Desktop", "Saved Here"));
    const noticed = await until(async () => (await page.locator('[data-testid="recent"] .row-name-text', { hasText: "Me at the zoo.mp3" }).count()) === 1, 20_000);
    const where = noticed ? await page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: "Me at the zoo.mp3" }).locator(".row-where").textContent() : null;
    record("A download saved outside Downloads appears in Recent downloads and is not moved", noticed && existsSync(elsewhere) && String(where).includes("Saved Here"), JSON.stringify({ noticed, stayed: existsSync(elsewhere), where }));

    // 3. Two downloads land (Messenger and DepEd streams) and are filed with their sources; an installer stays.
    fakeDownload("Grade 7 Quiz 2.docx", "Grade 7 Science Quiz Photosynthesis", "https://cdn.fbsbx.com/v/t59/quiz.docx?x=1", "https://www.facebook.com/messages/t/123");
    fakeDownload("DepEd Order 014.pdf", "%PDF-1.4 fake", "https://www.deped.gov.ph/wp-content/uploads/DO_014.pdf");
    writeFileSync(join(LAB, "Downloads", "printer-setup.exe"), "MZ");
    const quizFiled = await until(() => filed("Word", "Grade 7 Quiz 2.docx"), 40_000);
    const orderFiled = await until(() => filed("PDF", "DepEd Order 014.pdf"), 20_000);
    const exeStayed = existsSync(join(LAB, "Downloads", "printer-setup.exe"));
    // The rows first show the file where it landed (Downloads), then where it was filed.
    const rowWhere = async (name) => page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: name }).locator(".row-where").first().textContent().catch(() => "");
    const recentOk = await until(async () => {
      const metas = await page.locator('[data-testid="recent"] .row-meta').allTextContents();
      const sources = metas.some((m) => m.startsWith("from Messenger")) && metas.some((m) => m.startsWith("from DepEd"));
      return sources && (await rowWhere("Grade 7 Quiz 2.docx")).includes("School Files") && (await rowWhere("DepEd Order 014.pdf")).includes("School Files");
    }, 20_000);
    await capture("03-filed");
    record("Two downloads are filed by type and month with Messenger and DepEd sources and the rows show their new place; the installer stays in Downloads", quizFiled && orderFiled && exeStayed && recentOk, JSON.stringify({ quizFiled, orderFiled, exeStayed, recentOk }));

    // 3b. The Downloads promise (audit T-06, T-07, T-15), three scenarios side
    // by side: Undo of a filing leaves the file in Downloads with no second
    // Filed row; an old file that is edited and another that is renamed stay;
    // an empty Firefox placeholder beside its .part file waits, and is filed
    // once when the real download takes its name.
    const dataDir = join(LAB, "data");
    const sql = (statement) => execFileSync("python", ["-I", "-c", "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);r=c.execute(sys.argv[2]).fetchall();c.commit();print(r)", join(dataDir, "tote.sqlite"), statement], { encoding: "utf8" }).trim();
    const reading = fakeDownload("Reading List.docx", "reading list", "https://drive.google.com/file/d/abc/view");
    const readingFiled = await until(() => filed("Word", "Reading List.docx"), 40_000);
    const memo = join(LAB, "Downloads", "DepEd Memo 123.pdf");
    const memoPart = `${memo}.part`;
    writeFileSync(memo, "");
    writeFileSync(memoPart, "%PDF-1.4 half");
    const memoStarted = Date.now();
    const oldLesson = join(LAB, "Downloads", "Old Lesson.docx");
    const signedForm = join(LAB, "Downloads", "Existing Form signed.pdf");
    appendFileSync(oldLesson, " plus an edit saved from Word");
    renameSync(join(LAB, "Downloads", "Existing Form.pdf"), signedForm);
    const oldChanged = Date.now();
    await page.locator('[data-testid="nav-activity"]').click();
    await page.waitForSelector('[data-testid="activity-row"]', { timeout: STEP_TIMEOUT_MS });
    await page.locator('[data-testid="activity-row"]', { hasText: "Reading List.docx" }).first().locator('[data-testid="undo"]').click();
    const readingBack = await until(() => existsSync(reading), 15_000);
    const undoneAt = Date.now();
    await page.locator('[data-testid="nav-find"]').click();
    // 15 s after the placeholder appeared it is still empty, beside its part file, and not filed.
    await sleep(Math.max(0, memoStarted + 15_000 - Date.now()));
    const memoWaited = existsSync(memo) && existsSync(memoPart) && !filed("PDF", "DepEd Memo 123.pdf");
    // Firefox finishes: the part file gets the rest, takes the real name, then the stamp.
    writeFileSync(memoPart, "%PDF-1.4 the whole memo");
    renameSync(memoPart, memo);
    stampDownload(memo, "https://www.deped.gov.ph/wp-content/uploads/memo-123.pdf");
    // 25 s after the edit and the rename, and 15 s after the Undo, everything that is hers is still in Downloads.
    await sleep(Math.max(0, oldChanged + 25_000 - Date.now(), undoneAt + 15_000 - Date.now()));
    const readingStayed = existsSync(reading) && !filed("Word", "Reading List.docx");
    const readingRows = sql("SELECT status FROM ops WHERE kind = 'autoSort' AND src LIKE '%Reading List.docx' ORDER BY id");
    const oldStayed = existsSync(oldLesson) && existsSync(signedForm) && !filed("Word", "Old Lesson.docx") && !filed("PDF", "Existing Form signed.pdf");
    const memoFiled = await until(() => filed("PDF", "DepEd Memo 123.pdf"), 40_000);
    const memoOnce = memoFiled && !existsSync(memo) && !existsSync(memoPart) && !filed("PDF", "DepEd Memo 123 (2).pdf");
    const memoRows = sql("SELECT status FROM ops WHERE kind = 'autoSort' AND src LIKE '%DepEd Memo 123.pdf' ORDER BY id");
    await capture("03d-downloads-promise");
    record("Undo of a filing leaves the file in Downloads with no second Filed row; an old file edited and another renamed stay; an empty placeholder beside its .part file waits and is filed once",
      readingFiled && readingBack && readingStayed && readingRows === "[('undone',)]" && oldStayed && memoWaited && memoOnce && memoRows === "[('done',)]",
      JSON.stringify({ readingFiled, readingBack, readingStayed, readingRows, oldStayed, memoWaited, memoFiled, memoOnce, memoRows }));

    // 4. Move the quiz to Documents > Forms through the real folder picker and command.
    const quizRow = page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: "Grade 7 Quiz 2.docx" });
    await quizRow.locator("input[type=checkbox]").check();
    const barShown = await until(async () => (await page.locator('[data-testid="action-bar"]').count()) === 1);
    if (!barShown) {
      const state = await page.evaluate(() => ({
        checked: [...document.querySelectorAll('[data-testid="file-row"] input[type=checkbox]')].filter((c) => c.checked).length,
        rows: document.querySelectorAll('[data-testid="recent"] [data-testid="file-row"]').length,
        toasts: [...document.querySelectorAll(".toast")].map((t) => t.textContent),
      }));
      await capture("03b-no-action-bar");
      record("Ticking a row shows the action bar", false, JSON.stringify(state));
    }
    await capture("03c-before-move");
    const barBox = await page.evaluate(() => {
      const bar = document.querySelector('[data-testid="action-bar"]');
      const button = [...document.querySelectorAll('[data-testid="action-bar"] button')].find((b) => b.textContent?.includes("Move to"));
      const r = button?.getBoundingClientRect();
      const hit = r ? document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2) : null;
      return { bar: Boolean(bar), button: r ? { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) } : null, hitTag: hit?.tagName, hitClass: hit?.className, inner: window.innerHeight };
    });
    if (!barBox.bar || barBox.hitTag !== "BUTTON") record("The Move to... button is reachable and uncovered", false, JSON.stringify(barBox));
    await page.getByRole("button", { name: "Move to..." }).click({ timeout: 10_000 });
    await page.locator(".folder-option", { hasText: "Documents > Forms" }).click();
    await page.locator('[data-testid="move-confirm"]').click();
    const movedOnDisk = await until(() => existsSync(join(LAB, "Documents", "Forms", "Grade 7 Quiz 2.docx")));
    record("Move to... moves the file on disk", movedOnDisk, String(movedOnDisk));

    // 5. Rename it inline; the extension stays.
    await until(async () => (await page.locator('[data-testid="action-bar"]').count()) === 0);
    const row = page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: "Grade 7 Quiz 2.docx" });
    await row.locator("input[type=checkbox]").check();
    await page.getByRole("button", { name: "Rename" }).click();
    await page.locator('[data-testid="rename-input"]').fill("Science Quiz 2");
    await page.keyboard.press("Enter");
    const renamedOnDisk = await until(() => existsSync(join(LAB, "Documents", "Forms", "Science Quiz 2.docx")));
    record("Rename changes the name on disk and keeps .docx", renamedOnDisk, String(renamedOnDisk));

    // 6. Pin it; it shows under Pinned.
    await until(async () => (await page.locator('[data-testid="action-bar"]').count()) === 0);
    await page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: "Science Quiz 2.docx" }).getByRole("button", { name: /^Pin / }).click();
    const pinned = await until(async () => (await page.locator('[data-testid="pinned"] [data-testid="file-row"]', { hasText: "Science Quiz 2.docx" }).count()) === 1);
    record("Pin puts the file under Pinned", pinned, String(pinned));

    // 7. Recycle the DepEd order, then Undo from Activity restores it from the Recycle Bin.
    const orderRow = page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: "DepEd Order 014.pdf" });
    await orderRow.locator("input[type=checkbox]").check();
    await page.getByRole("button", { name: "Recycle Bin" }).click();
    await page.getByRole("button", { name: "Move to Recycle Bin" }).click();
    const gone = await until(() => !filed("PDF", "DepEd Order 014.pdf"));
    await page.locator('[data-testid="nav-activity"]').click();
    await page.waitForSelector('[data-testid="activity-row"]', { timeout: STEP_TIMEOUT_MS });
    const firstTitle = await page.locator('[data-testid="activity-row"] strong').first().textContent();
    await page.locator('[data-testid="activity-row"]').first().locator('[data-testid="undo"]').click();
    const back = await until(() => filed("PDF", "DepEd Order 014.pdf"), 15_000);
    await capture("04-activity");
    record("Recycle Bin removes the file and Undo restores it from the Recycle Bin", gone && firstTitle?.includes("Recycle Bin") && back, JSON.stringify({ gone, firstTitle, back }));

    // 7b. A file whose path is too long for the Recycle Bin is refused with one plain sentence and stays where it is.
    await page.locator('[data-testid="nav-find"]').click();
    await page.locator('[data-testid="search-input"]').fill("Deep Quiz");
    const deepRow = page.locator('[data-testid="results"] [data-testid="file-row"]', { hasText: "Deep Quiz.docx" }).first();
    const deepFound = await until(async () => (await deepRow.count()) === 1);
    let refusedText = null;
    if (deepFound) {
      await deepRow.locator("input[type=checkbox]").check();
      await page.getByRole("button", { name: "Recycle Bin" }).click();
      await page.getByRole("button", { name: "Move to Recycle Bin" }).click();
      refusedText = await page.locator(".toast-error .toast-text").first().textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
    }
    const deepStayed = existsSync(DEEP_FILE);
    await capture("04b-long-path-refused");
    record("A file whose path is too long for the Recycle Bin is refused with the plain sentence and stays where it is",
      deepFound && String(refusedText).includes("This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first.") && deepStayed,
      JSON.stringify({ deepFound, refusedText, deepStayed }));
    await page.locator('[data-testid="search-input"]').fill("");

    // 8. Clean up sees the duplicate lesson plans and the old download.
    await page.locator('[data-testid="nav-cleanup"]').click();
    await page.waitForSelector('[data-testid="cleanup-view"] .card', { timeout: STEP_TIMEOUT_MS });
    const dupes = await page.locator('[data-testid="cleanup-view"] .latest').count();
    const old = await page.locator('[data-testid="old-downloads-list"] .suggestion').count();
    await capture("05-cleanup");
    record("Clean up lists the duplicate lesson plans and the 90-day-old download", dupes >= 1 && old === 1, JSON.stringify({ dupes, old }));

    // 8b. Organize now on real files: grade and subject folders are made, the shortcut stays, Undo all reverses it.
    const school = join(LAB, "Documents", "School Files");
    await page.locator('[data-testid="organize-open"]').click();
    await page.waitForSelector('[data-testid="organize-dialog"]', { timeout: STEP_TIMEOUT_MS });
    const planTitle = await page.locator('[data-testid="organize-dialog"] #dialog-title').textContent();
    await page.locator('[data-testid="organize-confirm"]').click();
    const quizHome = join(school, "Grade 7", "Science", "Grade 7 Science Quiz.docx");
    const apHome = join(school, "Grade 8", "Araling Panlipunan", "AP 8 Reviewer.docx");
    const organized = await until(() => existsSync(quizHome) && existsSync(apHome), 15_000);
    const shortcutStayed = existsSync(join(LAB, "Desktop", "Zoom.lnk"));
    const desktopLeft = readdirSync(join(LAB, "Desktop")).filter((n) => !["Zoom.lnk", "Saved Here"].includes(n));
    await capture("05b-organized");
    await page.locator('[data-testid="nav-activity"]').click();
    await page.waitForSelector('[data-testid="activity-batch"]', { timeout: STEP_TIMEOUT_MS });
    await page.locator('[data-testid="undo-batch"]').first().click();
    const restored = await until(() => existsSync(join(LAB, "Desktop", "Grade 7 Science Quiz.docx")) && existsSync(join(LAB, "Desktop", "AP 8 Reviewer.docx")), 15_000);
    const emptyRemoved = await until(() => !existsSync(join(school, "Grade 7")) && !existsSync(join(school, "Grade 8")), 5_000);
    await capture("05c-organize-undone");
    record("Organize files real grade and subject names into new folders, leaves the shortcut, and Undo all puts every file back and removes the empty folders",
      organized && shortcutStayed && desktopLeft.length === 0 && restored && emptyRemoved,
      JSON.stringify({ planTitle, organized, shortcutStayed, desktopLeft, restored, emptyRemoved }));

    // 10. To do on the real store: a task due today, notes.txt attached through
    // the index search, then moved with Move to; the attachment follows.
    const todayText = await page.evaluate(() => {
      const d = new Date();
      return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    });
    await page.locator('[data-testid="nav-todo"]').click();
    await page.waitForSelector('[data-testid="todo-view"]', { timeout: STEP_TIMEOUT_MS });
    await page.locator('[data-testid="todo-input"]').fill("Submit SF2 for October");
    await page.locator('[data-testid="todo-due"]').fill(todayText);
    await page.locator('[data-testid="todo-add"]').click();
    const taskSaved = await until(async () => (await page.locator('[data-testid="task-row"]', { hasText: "Submit SF2 for October" }).count()) === 1);
    await page.locator('[data-testid="task-open"]').first().click();
    await page.locator('[data-testid="task-attach"]').click();
    await page.locator('[data-testid="attach-search"]').fill("santos");
    await page.locator('[data-testid="attach-option"]', { hasText: "notes.txt" }).click({ timeout: STEP_TIMEOUT_MS });
    const attachedNotes = await until(async () => (await page.locator('[data-testid="task-file"]', { hasText: "notes.txt" }).count()) === 1);
    await page.locator('[data-testid="nav-find"]').click();
    await page.locator('[data-testid="search-input"]').fill("santos");
    const notesRow = page.locator('[data-testid="results"] [data-testid="file-row"]', { hasText: "notes.txt" }).first();
    await notesRow.waitFor({ timeout: STEP_TIMEOUT_MS });
    await notesRow.locator("input[type=checkbox]").check();
    await page.getByRole("button", { name: "Move to..." }).click({ timeout: 10_000 });
    await page.locator(".folder-option", { hasText: "Documents > Forms" }).click();
    await page.locator('[data-testid="move-confirm"]').click();
    const notesMoved = await until(() => existsSync(join(LAB, "Documents", "Forms", "notes.txt")));
    await until(async () => (await page.locator('[data-testid="action-bar"]').count()) === 0);
    await page.locator('[data-testid="search-input"]').fill("");
    await page.locator('[data-testid="nav-todo"]').click();
    await page.waitForSelector('[data-testid="todo-view"]', { timeout: STEP_TIMEOUT_MS });
    if ((await page.locator('[data-testid="task-details"]').count()) === 0) await page.locator('[data-testid="task-open"]').first().click();
    const followed = await until(async () => ((await page.locator('[data-testid="task-file"] .task-file-where').first().textContent().catch(() => "")) ?? "").includes("Forms"));
    await capture("06-todo");
    record("To do on the real store: a task due today is saved, notes.txt attaches through search, and after Move to the attachment shows its new folder",
      taskSaved && attachedNotes && notesMoved && followed, JSON.stringify({ taskSaved, attachedNotes, notesMoved, followed }));

    // 11. A restart keeps the task; the start-up check saves the day's snapshot
    // and, from 07:00, shows the morning reminder and remembers the day.
    const testLog = join(process.env.LOCALAPPDATA ?? "", "com.focalstack.tote", "logs", "tote-test.log");
    const logSize = () => (existsSync(testLog) ? readFileSync(testLog, "utf8").length : 0);
    const logSince = (offset) => (existsSync(testLog) ? readFileSync(testLog, "utf8").slice(offset) : "");
    await stop(session);
    sql("DELETE FROM settings WHERE key IN ('tasks_snapshot_on', 'reminded_on')");
    const logStart = logSize();
    session = await launch(env);
    if (!session.page) throw new Error("could not attach after the restart");
    page = session.page;
    await page.locator('[data-testid="nav-todo"]').click();
    const survived = await until(async () => (await page.locator('[data-testid="task-row"]', { hasText: "Submit SF2 for October" }).count()) === 1);
    const snapshotLogged = await until(() => logSince(logStart).includes("tasks snapshot saved"), 15_000);
    const backupMade = existsSync(join(dataDir, "tasks.backup.sqlite"));
    const morning = new Date().getHours() >= 7;
    const reminded = morning ? await until(() => logSince(logStart).includes("morning reminder shown (1 due today"), 15_000) : null;
    const remindedOn = sql("SELECT value FROM settings WHERE key = 'reminded_on'");
    record(`After a restart the task is still there and the day's snapshot is saved; ${morning ? "the morning reminder was shown and the day remembered" : "before 07:00, so the reminder is not checked"}`,
      survived && snapshotLogged && backupMade && (!morning || (reminded && remindedOn.includes(todayText))),
      JSON.stringify({ survived, snapshotLogged, backupMade, reminded, remindedOn }));

    // 12. A damaged tasks.sqlite comes back from the daily snapshot with a toast,
    // and the damaged file is kept beside it.
    await stop(session);
    writeFileSync(join(dataDir, "tasks.sqlite"), "junk, not a database at all");
    session = await launch(env);
    if (!session.page) throw new Error("could not attach after damaging the list");
    page = session.page;
    const restoredToast = await until(async () => (await page.locator(".toast", { hasText: "restored from Tote's daily copy" }).count()) === 1, 10_000);
    await page.locator('[data-testid="nav-todo"]').click();
    const restoredTask = await until(async () => (await page.locator('[data-testid="task-row"]', { hasText: "Submit SF2 for October" }).count()) === 1);
    const damagedKept = readdirSync(dataDir).some((name) => name.startsWith("tasks.damaged-"));
    await capture("07-todo-restored");
    record("A damaged tasks.sqlite is restored from the daily snapshot: a toast says so, the task is back, and the damaged file is kept",
      restoredToast && restoredTask && damagedKept, JSON.stringify({ restoredToast, restoredTask, damagedKept }));

    // 9. Settings persist to the real database: notifications off survives a reload of app info.
    await page.locator('[data-testid="nav-settings"]').click();
    await page.waitForSelector('[data-testid="settings-view"]', { timeout: STEP_TIMEOUT_MS });
    const toggle = page.locator("label.toggle", { hasText: "Windows messages" }).locator("input");
    await toggle.uncheck();
    await sleep(500);
    const dbText = execFileSync("python", ["-I", "-c", `import sqlite3;print(sqlite3.connect(r'${join(LAB, "data", "tote.sqlite")}').execute("select value from settings where key='settings'").fetchone()[0])`], { encoding: "utf8" });
    record("Settings are written to the real database", dbText.includes('"notifications":false') && dbText.includes('"tourDone":true'), dbText.trim().slice(0, 120));

    // The window has no Windows frame: the green title bar carries the controls.
    const frame = await page.evaluate(() => ({ frameless: outerWidth === innerWidth && outerHeight === innerHeight, controls: [...document.querySelectorAll(".window-button")].map((b) => b.getAttribute("aria-label")) }));
    const startWidth = await page.evaluate(() => outerWidth);
    await page.getByRole("button", { name: "Maximize" }).click();
    const maximized = await until(() => page.evaluate(() => outerWidth === screen.availWidth));
    // The label follows Windows' resize event a moment after the width changes.
    await until(async () => (await page.locator(".window-button").nth(1).getAttribute("aria-label")) === "Restore");
    const restoreLabel = await page.locator(".window-button").nth(1).getAttribute("aria-label");
    await page.getByRole("button", { name: "Restore" }).click();
    const unmaximized = await until(() => page.evaluate((w) => outerWidth === w, startWidth));
    await capture("12-title-bar");
    record("The green title bar replaces the Windows frame; Maximize and Restore work from its buttons",
      frame.frameless && frame.controls.join(",") === "Minimize,Maximize,Close" && maximized && restoreLabel === "Restore" && unmaximized,
      JSON.stringify({ ...frame, maximized, restoreLabel, unmaximized }));

    // 13. A search answers while a large file is copied (audit T-12). The lab
    // has one drive, so TOTE_TEST_COPY_MOVES (test mode only) sends every move
    // through the copy between drives: the lock rule is what is tested.
    await stop(session);
    session = await launch({ ...env, TOTE_TEST_COPY_MOVES: "1" });
    if (!session.page) throw new Error("could not attach with copy moves on");
    page = session.page;
    const bigName = "Class Video.mp4";
    const bigSize = 300 * 1024 * 1024;
    const bigHome = join(LAB, "Desktop", bigName);
    const bigAway = join(LAB, "Documents", "Forms", bigName);
    writeFileSync(bigHome, Buffer.alloc(bigSize, 7));
    const partialsUnder = (folder) => (existsSync(folder) ? readdirSync(folder, { recursive: true }).map(String).filter((n) => n.split(/[\\/]/).pop().startsWith(".tote-partial-")) : []);
    const during = await page.evaluate(async ({ paths, destination }) => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      let settled = false;
      const move = invoke("move_files", { paths, destination }).then(() => "ok", (e) => JSON.stringify(e)).finally(() => { settled = true; });
      // Ask at once: on a fast disk the copy can finish in well under 150 ms.
      await new Promise((done) => setTimeout(done, 5));
      const movingAtStart = !settled;
      const started = performance.now();
      const found = await invoke("find", { query: "santos", filters: { kinds: [], sources: [] } }).then((r) => JSON.stringify(r).includes("notes.txt"), () => false);
      const searchMs = Math.round(performance.now() - started);
      return { movingAtStart, searchMs, found, movingAtEnd: !settled, moved: await move };
    }, { paths: [bigHome], destination: join(LAB, "Documents", "Forms") });
    const bigLanded = existsSync(bigAway) && !existsSync(bigHome) && statSync(bigAway).size === bigSize && partialsUnder(join(LAB, "Documents", "Forms")).length === 0;
    record("A search during a 300 MB move through the copy path answers within 2 s, and the file lands whole with no temporary copy left",
      during.movingAtStart && during.found && during.searchMs < 2000 && during.moved === "ok" && bigLanded,
      JSON.stringify({ ...during, bigLanded }));

    // 14. Tote closed in the middle of an Organize copy (audit T-14): after the
    // restart no change is left pending, the cut copy is gone, the original is
    // whole, and Activity says the change did not finish.
    renameSync(bigAway, bigHome);
    const plan = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("organize_plan"));
    const planned = plan.moves.find((m) => m.name === bigName);
    let cutSeen = false;
    if (planned) {
      await page.evaluate((paths) => { window.__organizeRun = window.__TAURI_INTERNALS__.invoke("organize_run", { paths }).catch(() => undefined); }, [planned.path]);
      const deadline = Date.now() + 15_000;
      while (!cutSeen && Date.now() < deadline) {
        cutSeen = partialsUnder(school).length > 0;
        if (!cutSeen) await sleep(10);
      }
      // Ended at once: taskkill as a separate process can land after a fast disk has finished the copy.
      if (cutSeen) session.app.kill();
    }
    await stop(session);
    const pendingBefore = sql("SELECT count(*) FROM ops WHERE status = 'pending'");
    session = await launch(env);
    if (!session.page) throw new Error("could not attach after closing Tote mid-Organize");
    page = session.page;
    const pendingAfter = sql("SELECT count(*) FROM ops WHERE status = 'pending'");
    const cutRow = sql(`SELECT status, message FROM ops WHERE kind = 'move' AND src LIKE '%${bigName}' ORDER BY id DESC LIMIT 1`);
    const cutLeft = partialsUnder(school);
    const originalWhole = existsSync(bigHome) && statSync(bigHome).size === bigSize;
    const shown = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("list_activity")).then((rows) => rows.some((r) => r.src.endsWith("Class Video.mp4") && r.status === "failed"));
    // When the close still landed after the move finished, the move must be whole and recorded as done.
    const bigDest = join(planned ? planned.destDir : school, bigName);
    const landedWhole = !existsSync(bigHome) && existsSync(bigDest) && statSync(bigDest).size === bigSize;
    const cutOk = (cutRow.startsWith("[('failed', 'Tote was closed before this finished.") && originalWhole && shown) || (cutRow.startsWith("[('done'") && landedWhole);
    rmSync(bigHome, { force: true });
    rmSync(bigDest, { force: true });
    record("Tote closed in the middle of an Organize copy leaves no pending row after the restart, no cut copy, the original whole, and a failed row in Activity",
      Boolean(planned) && cutSeen && pendingBefore === "[(1,)]" && pendingAfter === "[(0,)]" && cutLeft.length === 0 && cutOk,
      JSON.stringify({ planned: Boolean(planned), cutSeen, pendingBefore, pendingAfter, cutRow, cutLeft, originalWhole, landedWhole, shown }));

    if (process.env.TOTE_CHECK_UPDATE) {
      // The live manifest, the signed installer on the site and the public key
      // in this build, end to end. The renderer's own IPC bridge is used, as the
      // banner would.
      const offered = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("check_update"));
      const installed = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("install_update").then(() => "ok", (e) => JSON.stringify(e)));
      const logPath = join(process.env.LOCALAPPDATA ?? "", "com.focalstack.tote", "logs", "tote-test.log");
      const log = existsSync(logPath) ? readFileSync(logPath, "utf8") : "";
      const verified = /update: \S+ downloaded \(\d+ bytes\) and its signature verified/.test(log) && log.includes("dry run, so");
      record("Update: the live manifest offers a release, its installer downloads from codewithlex.site and its signature verifies; nothing is installed in a dry run",
        Boolean(offered?.version) && installed === "ok" && verified, JSON.stringify({ offered, installed, verified }));
    }

    record("No uncaught page errors in the real window", pageErrors.length === 0, pageErrors.join(" | "));
  } catch (error) {
    process.stderr.write(`[NATIVE CHECK ERROR] ${error.stack || error.message}\n`);
    exitCode = 1;
  } finally {
    await stop(session);
    const logPath = join(process.env.LOCALAPPDATA ?? "", "com.focalstack.tote", "logs", "tote-test.log");
    if (existsSync(logPath)) writeFileSync(join(EVIDENCE_DIR, "tote.log"), readFileSync(logPath, "utf8").split("\n").slice(-200).join("\n"));
    writeFileSync(join(EVIDENCE_DIR, "summary.json"), `${JSON.stringify({ stamp, lab: LAB, steps: results, pageErrors, downloadsLeft: readdirSync(join(LAB, "Downloads")) }, null, 2)}\n`);
  }
  const failed = results.filter((r) => !r.passed).length;
  if (failed > 0 || pageErrors.length > 0) exitCode = 1;
  process.stdout.write(`\nEvidence written to ${EVIDENCE_DIR}\n${exitCode === 0 ? "[NATIVE CHECK PASS] The real app files, finds, moves, renames, pins, recycles and undoes." : `[NATIVE CHECK FAIL] ${failed} step(s) failed.`}\n`);
  process.exit(exitCode);
}

await main();
