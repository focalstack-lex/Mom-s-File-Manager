// To do (spec 2026-10-14): what needs doing, grouped by when it is due, with
// the files that go with each task. The one primary action is Add.

import { ChevronRight, ListChecks } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { ShowMore } from "../components/ShowMore";
import { SkeletonList } from "../components/Skeleton";
import { TaskRow } from "../components/TaskRow";
import { PAGE, takeRows, useShown } from "../lib/paging";
import { groupTasks } from "../lib/todo";
import type { AppInfo } from "../lib/types";
import { useApp } from "../store/app";
import { useTodo } from "../store/todo";

export function TodoView({ info }: { info: AppInfo }) {
  const tasks = useTodo((s) => s.tasks);
  const load = useTodo((s) => s.load);
  const add = useTodo((s) => s.add);
  const saveSettings = useApp((s) => s.saveSettings);
  const [title, setTitle] = useState("");
  const [due, setDue] = useState("");
  const [doneOpen, setDoneOpen] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  // A store that could not be opened (info.tasksUnavailable) has nothing to load.
  useEffect(() => {
    if (!info.tasksUnavailable) void load();
  }, [load, info.tasksUnavailable]);

  // Opening the list once retires the sidebar's New marker.
  useEffect(() => {
    if (!info.settings.todoSeen) void saveSettings({ ...info.settings, todoSeen: true });
  }, [info.settings, saveSettings]);

  const now = new Date();
  const open = (tasks ?? []).filter((t) => t.doneAt === null);
  const done = (tasks ?? []).filter((t) => t.doneAt !== null).sort((a, b) => (b.doneAt ?? 0) - (a.doneAt ?? 0));
  const groups = groupTasks(open, now);
  const [shown, showMore] = useShown(PAGE.todo, PAGE.todo, "todo");
  const visible = takeRows(groups, shown);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (title.trim() === "") return;
    if (await add(title, due === "" ? null : due)) {
      setTitle("");
      setDue("");
    }
    input.current?.focus();
  };

  return (
    <div className="main-inner" data-testid="todo-view">
      <div className="view-head">
        <h1 className="view-title">To do</h1>
        <p className="view-lead">What needs doing, with its files. Tote reminds you on the morning it is due.</p>
      </div>
      <form className="card todo-add" onSubmit={(e) => void submit(e)} aria-label="Add a task">
        <label className="visually-hidden" htmlFor="todo-title">
          What needs doing?
        </label>
        <input id="todo-title" ref={input} className="todo-add-title" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="What needs doing? For example: Submit SF2 for October" maxLength={200} autoComplete="off" disabled={info.tasksUnavailable} data-testid="todo-input" />
        <label className="todo-due">
          Due
          <input type="date" value={due} onChange={(e) => setDue(e.target.value)} aria-label="Due date, optional" disabled={info.tasksUnavailable} data-testid="todo-due" />
        </label>
        <button type="submit" className="button button-primary" disabled={info.tasksUnavailable || title.trim() === ""} data-testid="todo-add">
          Add
        </button>
      </form>
      {info.tasksUnavailable ? (
        <div className="card empty" role="alert" data-testid="todo-unavailable">
          <strong>Your to-do list could not be opened.</strong>
          Restart Tote. If this keeps happening, tell Lex.
        </div>
      ) : tasks === null ? (
        <SkeletonList label="Loading your list" rows={3} />
      ) : tasks.length === 0 ? (
        <div className="card empty" data-testid="todo-empty">
          <span className="empty-mark" aria-hidden>
            <ListChecks size={26} />
          </span>
          <strong>Nothing to do yet</strong>
          Add your first task above.
        </div>
      ) : (
        <>
          {open.length === 0 && <p className="status-line">Everything on your list is done.</p>}
          {visible.map((group, i) => (
            <section key={group.title} className="todo-group" aria-labelledby={`todo-group-${i}`} data-testid="todo-group">
              <h2 id={`todo-group-${i}`} className={`todo-group-title${group.title === "Overdue" ? " is-overdue" : ""}`}>
                {group.title}
              </h2>
              <div className="rows">
                {group.items.map((task) => (
                  <TaskRow key={task.id} task={task} info={info} now={now} />
                ))}
              </div>
            </section>
          ))}
          <ShowMore shown={Math.min(shown, open.length)} total={open.length} step={PAGE.todo} onMore={showMore} testId="todo-more" />
          {done.length > 0 && (
            <section className="card fold-group" aria-labelledby="todo-done-title" data-testid="todo-done">
              <div className={`fold${doneOpen ? " is-open" : ""}`}>
                <h2 className="fold-title" id="todo-done-title">
                  <button type="button" className="fold-toggle" aria-expanded={doneOpen} aria-controls="todo-done-body" onClick={() => setDoneOpen((v) => !v)} data-testid="todo-done-toggle">
                    <span className="fold-name">Done</span>
                    <span className="count-pill">{done.length}</span>
                    <ChevronRight className="fold-chevron" size={20} aria-hidden />
                  </button>
                </h2>
                {doneOpen && (
                  <div id="todo-done-body" className="rows">
                    {done.map((task) => (
                      <TaskRow key={task.id} task={task} info={info} now={now} />
                    ))}
                  </div>
                )}
              </div>
            </section>
          )}
        </>
      )}
    </div>
  );
}
