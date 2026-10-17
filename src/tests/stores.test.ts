// The stores against a fake native side: what each one records when a call
// fails, so a screen can show one plain sentence instead of a loading state,
// and what an Undo may claim.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MockBackend } from "../lib/ipc";
import type { Task, ToteError } from "../lib/types";

// The Tauri packages are never reached: ipc.ts routes to window.__toteMock.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const answers = new Map<string, () => unknown>();
const mock: MockBackend = {
  async invoke(command) {
    const answer = answers.get(command);
    if (!answer) throw { code: "unsupported", message: `no answer for ${command}` } satisfies ToteError;
    return answer();
  },
  listen: () => () => undefined,
};
// vitest runs these in node; the stores only need window for the mock and timers.
Object.assign(globalThis, { window: Object.assign(globalThis, { __toteMock: mock }) });

const { movedOps, stampNote, useApp } = await import("../store/app");
const { useFind } = await import("../store/find");
const { useTodo } = await import("../store/todo");

const RAW = "The process cannot access the file because it is being used by another process. (os error 32)";
const task = (id: number): Task => ({ id, title: `Task ${id}`, due: null, repeat: "never", doneAt: null, createdAt: id, files: [] });
const errorToasts = () => useApp.getState().toasts.filter((t) => t.kind === "error").map((t) => t.text);

beforeEach(() => {
  answers.clear();
  useApp.setState({ info: null, infoError: null, toasts: [], browsers: null, browsersError: null });
  useFind.setState({ result: null, failed: false, loading: false });
  useTodo.setState({ tasks: null, loadError: null });
});

describe("app store", () => {
  it("records a failed first load as a sentence for the shell, not a toast", async () => {
    answers.set("get_app_info", () => {
      throw { code: "db", message: RAW } satisfies ToteError;
    });
    await useApp.getState().loadInfo();
    expect(useApp.getState().infoError).toBe("Tote could not read its list right now. Try again in a moment.");
    expect(useApp.getState().toasts).toHaveLength(0);
  });

  it("says whether a settings save was kept", async () => {
    answers.set("set_settings", () => {
      throw { code: "db", message: "Tote could not save that. Try again in a moment." } satisfies ToteError;
    });
    const settings = { startAtLogin: true, notifications: true, tourDone: true, schoolYear: null, quarters: [], extraFolders: [], excludedFolders: [], subjectRules: [], checkUpdates: true, todoSeen: true };
    expect(await useApp.getState().saveSettings(settings)).toBe(false);
    expect(errorToasts()).toEqual(["Tote could not save that. Try again in a moment."]);
    answers.set("set_settings", () => settings);
    expect(await useApp.getState().saveSettings(settings)).toBe(true);
  });

  it("records a browser check that could not run as a sentence, never as no browsers found", async () => {
    answers.set("browser_check", () => {
      throw { code: "io", message: RAW } satisfies ToteError;
    });
    await useApp.getState().checkBrowsers();
    expect(useApp.getState()).toMatchObject({ browsers: null, browsersError: "Windows could not finish that. Try again in a moment." });
    expect(useApp.getState().toasts).toHaveLength(0);
    answers.set("browser_check", () => []);
    await useApp.getState().checkBrowsers();
    expect(useApp.getState()).toMatchObject({ browsers: [], browsersError: null });
  });
});

describe("move toasts", () => {
  const op = (id: number, dst: string, stampLost: boolean) => ({ id, kind: "move", src: `C:\\u\\Downloads\\${id}.pdf`, dst, status: "done", message: null, automatic: false, createdAt: id, batch: null, stampLost });

  it("finds the rows of the files just moved and says when one lost its web mark (audit T-43)", async () => {
    answers.set("list_activity", () => [op(1, "E:\\a.pdf", true), op(2, "E:\\b.pdf", false), op(3, "C:\\c.pdf", true)]);
    const ops = await movedOps(["E:\\a.pdf", "E:\\b.pdf"]);
    expect(ops.map((o) => o.id)).toEqual([1, 2]);
    expect(stampNote(ops)).toBe(" Windows will no longer mark 1 of them as from the internet.");
    expect(stampNote(ops.slice(0, 1))).toBe(" Windows will no longer mark it as from the internet.");
    expect(stampNote(ops.slice(1))).toBe("");
    expect(stampNote([ops[0], { ...ops[0], id: 9 }])).toBe(" Windows will no longer mark them as from the internet.");
  });
});

describe("toasts", () => {
  // Audit T-56: a toast's Undo used to vanish after 10 s even under the pointer.
  it("waits while the pointer is over a toast or focus is in it, then runs out the rest of its time", () => {
    vi.useFakeTimers();
    try {
      useApp.getState().toast("Moved to School Files.", "info", [7]);
      const id = useApp.getState().toasts[0].id;
      vi.advanceTimersByTime(4000);
      useApp.getState().holdToast(id);
      useApp.getState().holdToast(id); // pointer and focus both
      vi.advanceTimersByTime(60_000);
      expect(useApp.getState().toasts.map((t) => t.id)).toEqual([id]);
      useApp.getState().releaseToast(id);
      vi.advanceTimersByTime(60_000);
      expect(useApp.getState().toasts).toHaveLength(1);
      useApp.getState().releaseToast(id);
      vi.advanceTimersByTime(5900);
      expect(useApp.getState().toasts).toHaveLength(1);
      vi.advanceTimersByTime(200);
      expect(useApp.getState().toasts).toHaveLength(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps a failed install's sentence for the banner and Settings, the same words as its toast", async () => {
    const sentence = "The update could not be installed. Try again later.";
    answers.set("install_update", () => {
      throw { code: "updateInstall", message: sentence } satisfies ToteError;
    });
    await useApp.getState().installUpdate();
    expect(useApp.getState()).toMatchObject({ updating: false, updateError: sentence });
    expect(errorToasts()).toEqual([sentence]);
    answers.set("install_update", () => undefined);
    const again = useApp.getState().installUpdate();
    expect(useApp.getState().updateError).toBeNull();
    await again;
  });
});

describe("find store", () => {
  it("keeps the last answer when a search fails and says so without a toast", async () => {
    const answer = { groups: [], pinned: [], orphanPins: [], recent: [], index: { scanning: false, scanned: 1, total: 1, files: 1 } };
    answers.set("find", () => answer);
    await useFind.getState().refresh();
    expect(useFind.getState().result).toEqual(answer);
    answers.set("find", () => {
      throw { code: "db", message: RAW } satisfies ToteError;
    });
    await useFind.getState().refresh();
    expect(useFind.getState()).toMatchObject({ result: answer, failed: true, loading: false });
    expect(useApp.getState().toasts).toHaveLength(0);
    answers.set("find", () => answer);
    await useFind.getState().refresh();
    expect(useFind.getState().failed).toBe(false);
  });
});

describe("todo store", () => {
  it("records a failed load as a sentence for the screen, not a toast", async () => {
    answers.set("list_tasks", () => {
      throw { code: "db", message: RAW } satisfies ToteError;
    });
    await useTodo.getState().load();
    expect(useTodo.getState()).toMatchObject({ tasks: null, loadError: "Tote could not read its list right now. Try again in a moment." });
    expect(useApp.getState().toasts).toHaveLength(0);
  });

  it("returns whether reopen and restore worked, so the toast can be honest", async () => {
    answers.set("list_tasks", () => [task(1)]);
    answers.set("reopen_task", () => undefined);
    answers.set("restore_task", () => {
      throw { code: "validation", message: "That task is no longer on the list. Open To do again to check." } satisfies ToteError;
    });
    expect(await useTodo.getState().reopen(1)).toBe(true);
    expect(await useTodo.getState().restore(1)).toBe(false);
    expect(errorToasts()).toEqual(["That task is no longer on the list. Open To do again to check."]);
    expect(useTodo.getState().tasks).toHaveLength(1);
  });
});
