// Files dragged over the window: the screen shows the drop card only while
// files (not text or a link) are over it, and hides it on a drop or when
// the drag leaves. The drop itself is filed by the native side (lib.rs).

import { afterEach, describe, expect, it, vi } from "vitest";

type Payload = { type: "enter"; paths: string[] } | { type: "over" } | { type: "drop"; paths: string[] } | { type: "leave" };
let fire: (payload: Payload) => void = () => undefined;

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (handler: (e: { payload: Payload }) => void) => {
      fire = (payload) => handler({ payload });
      return Promise.resolve(() => undefined);
    },
  }),
}));

Object.assign(globalThis, { window: globalThis });
const { onFileDrag, MOCK_DRAG_EVENT } = await import("../lib/ipc");

describe("onFileDrag", () => {
  afterEach(() => {
    delete window.__toteMock;
  });

  it("is on while files are over the window and off on a drop or a leave", async () => {
    const seen: boolean[] = [];
    await onFileDrag((over) => seen.push(over));
    fire({ type: "enter", paths: ["C:\\Users\\Her\\Downloads\\Quiz.docx"] });
    fire({ type: "over" });
    fire({ type: "drop", paths: ["C:\\Users\\Her\\Downloads\\Quiz.docx"] });
    fire({ type: "enter", paths: ["C:\\Users\\Her\\Downloads\\Quiz.docx"] });
    fire({ type: "leave" });
    expect(seen).toEqual([true, false, true, false]);
  });

  it("stays off for a drag that carries no files", async () => {
    const seen: boolean[] = [];
    await onFileDrag((over) => seen.push(over));
    fire({ type: "enter", paths: [] });
    fire({ type: "over" });
    expect(seen).toEqual([false]);
  });

  it("hears the harness's stand-in in a plain browser", async () => {
    const handlers: ((payload: unknown) => void)[] = [];
    window.__toteMock = {
      invoke: () => Promise.resolve(null),
      listen: (event, handler) => {
        expect(event).toBe(MOCK_DRAG_EVENT);
        handlers.push(handler);
        return () => undefined;
      },
    };
    const seen: boolean[] = [];
    await onFileDrag((over) => seen.push(over));
    handlers[0](true);
    handlers[0](false);
    expect(seen).toEqual([true, false]);
  });
});
