"use client";

import { useCallback, useEffect, useState } from "react";
import { invoker } from "@/lib/tauri";

type Reminder = { kind: string; title: string; body: string };

/**
 * The reminder window (Tauri window label `reminder`).
 *
 * This exists because an OS notification cannot promise to stay on screen: on
 * macOS it is a banner the system clears after a few seconds, which is useless
 * for a break reminder aimed at someone who has walked away from the desk. This
 * window is borderless and always-on-top, and nothing removes it but the user.
 *
 * It is deliberately self-contained — no session, no queries, no navigation.
 * The only ways out are the buttons.
 */
export default function ReminderPage() {
  const [reminder, setReminder] = useState<Reminder | null>(null);
  const [busy, setBusy] = useState(false);

  // The window is reused across reminders, so poll rather than read once: a
  // second reminder re-points this same page at new text.
  useEffect(() => {
    let alive = true;
    const read = async () => {
      try {
        const invoke = await invoker();
        const next = await invoke<Reminder | null>("current_reminder");
        if (alive) setReminder(next);
      } catch {
        // The window is up; leaving the last text on screen beats blanking it.
      }
    };
    void read();
    const id = setInterval(read, 2000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  // A reminder that appears in silence behind whatever the user was doing is
  // half a reminder. Synthesised rather than bundled so it costs no asset.
  useEffect(() => {
    try {
      const Ctx =
        window.AudioContext ??
        (window as unknown as { webkitAudioContext?: typeof AudioContext })
          .webkitAudioContext;
      if (!Ctx) return;
      const ctx = new Ctx();
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.frequency.value = 660;
      gain.gain.setValueAtTime(0.0001, ctx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.12, ctx.currentTime + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + 0.45);
      osc.connect(gain).connect(ctx.destination);
      osc.start();
      osc.stop(ctx.currentTime + 0.5);
      osc.onended = () => void ctx.close();
    } catch {
      // Audio is a nicety; the window is the reminder.
    }
  }, []);

  // Ignore activation for a moment after the window appears. A reminder can
  // land while the user is mid-click or mid-keystroke somewhere else, and an
  // input already in flight must not be able to dismiss a prompt nobody has
  // read yet. Standard practice for anything that appears unprompted.
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    const id = setTimeout(() => setArmed(true), 700);
    return () => clearTimeout(id);
  }, []);

  const run = useCallback(async (command: string) => {
    setBusy(true);
    try {
      const invoke = await invoker();
      await invoke(command);
    } finally {
      // No setBusy(false) on success — the window is closing.
      setBusy(false);
    }
  }, []);

  const isBreak = reminder?.kind === "break";

  return (
    <main
      data-tauri-drag-region
      className="flex h-screen select-none flex-col justify-between rounded-xl border border-slate-200 bg-white p-5 dark:border-slate-800 dark:bg-slate-900"
    >
      <div data-tauri-drag-region className="flex items-start gap-3">
        <span
          aria-hidden
          className={`mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-lg text-lg ${
            isBreak
              ? "bg-blue-100 text-blue-700 dark:bg-blue-950/60 dark:text-blue-300"
              : "bg-amber-100 text-amber-700 dark:bg-amber-950/60 dark:text-amber-300"
          }`}
        >
          {isBreak ? "☕" : "⏱"}
        </span>
        <div data-tauri-drag-region className="min-w-0">
          <h1 className="text-[15px] font-semibold leading-tight text-slate-900 dark:text-slate-100">
            {reminder?.title ?? "TimeTracker"}
          </h1>
          <p className="mt-1 text-[13px] leading-snug text-slate-600 dark:text-slate-400">
            {reminder?.body ?? ""}
          </p>
        </div>
      </div>

      <div className="flex items-center justify-end gap-2">
        {isBreak && (
          <button
            type="button"
            disabled={busy || !armed}
            onClick={() => void run("mute_break_reminders")}
            className="rounded-md px-3 py-2 text-[13px] font-medium text-slate-600 hover:bg-slate-100 disabled:opacity-50 dark:text-slate-400 dark:hover:bg-slate-800"
          >
            Don&apos;t remind me
          </button>
        )}
        {/* No autoFocus: the window does not take keyboard focus, and a default
            button here would let a stray Enter dismiss an unread reminder. */}
        <button
          type="button"
          disabled={busy || !armed}
          onClick={() => void run("dismiss_reminder")}
          className="min-w-[90px] rounded-md bg-slate-900 px-4 py-2 text-[13px] font-semibold text-white hover:bg-slate-800 disabled:opacity-50 dark:bg-slate-100 dark:text-slate-900 dark:hover:bg-white"
        >
          Ok
        </button>
      </div>
    </main>
  );
}
