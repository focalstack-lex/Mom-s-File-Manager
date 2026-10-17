// The To do list (spec 2026-10-14): tasks from the native side, which one is
// open, and actions that offer Undo in a toast instead of asking first.

import { create } from "zustand";
import { describeError } from "../lib/errors";
import { ipc, logRenderer } from "../lib/ipc";
import { dayWords } from "../lib/todo";
import type { Repeat, Task } from "../lib/types";
import { useApp } from "./app";

interface TodoStore {
  tasks: Task[] | null;
  /** The last load failed; the screen shows this sentence with Try again. */
  loadError: string | null;
  /** The task whose details are open, one at a time. */
  openId: number | null;
  load(): Promise<void>;
  setOpen(id: number | null): void;
  add(title: string, due: string | null): Promise<boolean>;
  update(task: Task, patch: { title?: string; due?: string | null; repeat?: Repeat }): Promise<void>;
  complete(task: Task): Promise<void>;
  /** True when the task is back on the list; a toast may say so only then. */
  reopen(id: number): Promise<boolean>;
  remove(task: Task): Promise<void>;
  /** True when the task is back on the list; a toast may say so only then. */
  restore(id: number): Promise<boolean>;
  attach(id: number, path: string): Promise<boolean>;
  detach(id: number, path: string): Promise<void>;
}

const fail = (error: unknown) => useApp.getState().fail(error);

export const useTodo = create<TodoStore>((set, get) => ({
  tasks: null,
  loadError: null,
  openId: null,
  load: async () => {
    try {
      set({ tasks: await ipc.tasks(), loadError: null });
    } catch (error) {
      // The screen shows the sentence in place with Try again, not a toast
      // over a skeleton that never ends.
      console.warn("Tote:", error);
      logRenderer("error", describeError(error));
      set({ loadError: describeError(error) });
    }
  },
  setOpen: (openId) => set({ openId }),
  add: async (title, due) => {
    try {
      await ipc.addTask(title, due);
      await get().load();
      return true;
    } catch (error) {
      fail(error);
      return false;
    }
  },
  update: async (task, patch) => {
    const due = patch.due !== undefined ? patch.due : task.due;
    // Without a date nothing can repeat, so clearing the date stops the repeat.
    const repeat = due === null ? "never" : (patch.repeat ?? task.repeat);
    try {
      await ipc.updateTask(task.id, patch.title ?? task.title, due, repeat);
    } catch (error) {
      fail(error);
    }
    await get().load();
  },
  complete: async (task) => {
    try {
      const done = await ipc.completeTask(task.id);
      await get().load();
      const text = done.next?.due ? `Done. Next one is due ${dayWords(done.next.due)}.` : done.noNextQuarter ? "Done. There is no next quarter in Settings, so it will not repeat." : "Done.";
      useApp.getState().toast(text, "info", undefined, undefined, { action: "reopen", id: task.id });
    } catch (error) {
      fail(error);
    }
  },
  reopen: async (id) => {
    let ok = true;
    try {
      await ipc.reopenTask(id);
    } catch (error) {
      ok = false;
      fail(error);
    }
    await get().load();
    return ok;
  },
  remove: async (task) => {
    try {
      await ipc.deleteTask(task.id);
      if (get().openId === task.id) set({ openId: null });
      await get().load();
      useApp.getState().toast("Task deleted.", "info", undefined, undefined, { action: "restore", id: task.id });
    } catch (error) {
      fail(error);
    }
  },
  restore: async (id) => {
    let ok = true;
    try {
      await ipc.restoreTask(id);
    } catch (error) {
      ok = false;
      fail(error);
    }
    await get().load();
    return ok;
  },
  attach: async (id, path) => {
    try {
      await ipc.attachFile(id, path);
      await get().load();
      return true;
    } catch (error) {
      fail(error);
      return false;
    }
  },
  detach: async (id, path) => {
    try {
      await ipc.detachFile(id, path);
    } catch (error) {
      fail(error);
    }
    await get().load();
  },
}));
