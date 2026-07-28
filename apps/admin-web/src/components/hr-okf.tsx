"use client";

import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { NotebookPen, X } from "lucide-react";
import { getOkf, updateOkf } from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";

/**
 * Floating "company rulebook (OKF)" button, mounted on every admin screen but
 * rendered only for HR. Opens a right-side drawer that shows the rulebook and
 * lets HR edit + save it. The document is the single source of truth in the DB
 * (served by `/admin/okf`); no markdown renderer is bundled, so it's shown as
 * the raw markdown that HR edits directly.
 */
export function HrOkf() {
  const { user } = useAdminSession();
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [err, setErr] = useState("");
  const qc = useQueryClient();

  const isHr = user?.role === "hr";

  const doc = useQuery({
    queryKey: ["okf"],
    queryFn: getOkf,
    enabled: open && isHr,
  });

  const save = useMutation({
    mutationFn: () => updateOkf(draft),
    onSuccess: () => {
      setErr("");
      setEditing(false);
      qc.invalidateQueries({ queryKey: ["okf"] });
    },
    onError: (e) => setErr(e instanceof Error ? e.message : "Failed to save."),
  });

  // Escape closes the drawer (but not while editing, to avoid losing changes).
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !editing) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, editing]);

  if (!isHr) return null;

  function startEdit() {
    setDraft(doc.data?.content ?? "");
    setErr("");
    setEditing(true);
  }

  function close() {
    if (editing && draft !== (doc.data?.content ?? "")) {
      if (!window.confirm("Discard unsaved changes?")) return;
    }
    setOpen(false);
    setEditing(false);
    setErr("");
  }

  const edited = doc.data?.updated_at
    ? `Last edited ${new Date(doc.data.updated_at).toLocaleString()}${
        doc.data.updated_by_name ? ` by ${doc.data.updated_by_name}` : ""
      }`
    : "Single source of truth for company policy";

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        title="Company rulebook (OKF)"
        aria-label="Open company rulebook"
        className="fixed bottom-6 right-6 z-[60] flex h-12 w-12 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition hover:opacity-90"
      >
        <NotebookPen className="h-5 w-5" />
      </button>

      {open && (
        <div
          className="fixed inset-0 z-[70] flex justify-end bg-black/50"
          onClick={close}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            className="flex h-full w-full max-w-2xl flex-col bg-card shadow-xl"
          >
            <div className="flex items-center justify-between gap-3 border-b px-5 py-4">
              <div className="min-w-0">
                <h2 className="text-base font-semibold">Company Rulebook — OKF</h2>
                <p className="truncate text-xs text-muted-foreground">{edited}</p>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                {!editing && (
                  <button
                    type="button"
                    onClick={startEdit}
                    disabled={doc.isLoading || doc.isError}
                    className="rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
                  >
                    Edit
                  </button>
                )}
                <button
                  type="button"
                  onClick={close}
                  aria-label="Close"
                  className="rounded-md p-1.5 text-muted-foreground hover:bg-secondary"
                >
                  <X className="h-5 w-5" />
                </button>
              </div>
            </div>

            <div className="flex min-h-0 flex-1 flex-col p-5">
              {doc.isLoading ? (
                <p className="text-sm text-muted-foreground">Loading…</p>
              ) : doc.isError ? (
                <p className="text-sm text-red-600">Could not load the rulebook.</p>
              ) : editing ? (
                <textarea
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  spellCheck={false}
                  className="min-h-0 flex-1 w-full resize-none rounded-md border border-input bg-background p-3 font-mono text-xs leading-relaxed focus:outline-none focus:ring-2 focus:ring-ring"
                />
              ) : (
                <div className="min-h-0 flex-1 overflow-y-auto rounded-md border bg-background p-3">
                  <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-foreground">
                    {doc.data?.content}
                  </pre>
                </div>
              )}
            </div>

            {editing && (
              <div className="flex items-center justify-between gap-3 border-t px-5 py-3">
                <p className="min-w-0 flex-1 truncate text-xs text-red-600">{err}</p>
                <div className="flex shrink-0 gap-2">
                  <button
                    type="button"
                    onClick={() => {
                      setEditing(false);
                      setErr("");
                    }}
                    className="rounded-md bg-secondary px-4 py-2 text-sm font-medium hover:opacity-90"
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    onClick={() => save.mutate()}
                    disabled={save.isPending || !draft.trim()}
                    className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
                  >
                    {save.isPending ? "Saving…" : "Save"}
                  </button>
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </>
  );
}
