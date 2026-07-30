"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, NotebookPen, Plus, Trash2, Upload, X } from "lucide-react";
import {
  listPolicies,
  getPolicy,
  createPolicy,
  updatePolicy,
  deletePolicy,
  uploadPolicyFile,
  getPolicyDownloadUrl,
  type PolicySummary,
} from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";

const SYSTEM_SLUG = "system-rulebook";

/**
 * Floating "company handbook (OKF)" button, on every admin screen but only for
 * HR. Opens a drawer that lists every policy document grouped by category and
 * lets HR view / edit / create / delete them. The library is the source of truth
 * in the DB (`/policies` + `/admin/policies`); employees get a read-only view in
 * their own app. No markdown renderer is bundled, so content shows as raw
 * markdown that HR edits directly.
 */
export function HrOkf() {
  const { user } = useAdminSession();
  const isHr = user?.role === "hr";
  const qc = useQueryClient();

  const [open, setOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState({ title: "", category: "", content: "" });
  const [creating, setCreating] = useState(false);
  const [err, setErr] = useState("");

  const list = useQuery({
    queryKey: ["policies"],
    queryFn: listPolicies,
    enabled: open && isHr,
  });

  const doc = useQuery({
    queryKey: ["policy", selectedId],
    queryFn: () => getPolicy(selectedId as string),
    enabled: open && isHr && !!selectedId,
  });

  // Group the list by category, preserving server order.
  const grouped = useMemo(() => {
    const g: Record<string, PolicySummary[]> = {};
    for (const d of list.data ?? []) (g[d.category] ??= []).push(d);
    return Object.entries(g);
  }, [list.data]);

  const save = useMutation({
    mutationFn: async () => {
      const input = {
        title: draft.title.trim(),
        category: draft.category.trim() || "General",
        content: draft.content,
      };
      return creating ? createPolicy(input) : updatePolicy(selectedId as string, input);
    },
    onSuccess: (saved) => {
      setErr("");
      setEditing(false);
      setCreating(false);
      setSelectedId(saved.id);
      qc.invalidateQueries({ queryKey: ["policies"] });
      qc.invalidateQueries({ queryKey: ["policy", saved.id] });
    },
    onError: (e) => setErr(e instanceof Error ? e.message : "Failed to save."),
  });

  const remove = useMutation({
    mutationFn: () => deletePolicy(selectedId as string),
    onSuccess: () => {
      setSelectedId(null);
      setEditing(false);
      qc.invalidateQueries({ queryKey: ["policies"] });
    },
    onError: (e) => setErr(e instanceof Error ? e.message : "Failed to delete."),
  });

  const fileInputRef = useRef<HTMLInputElement>(null);
  const upload = useMutation({
    mutationFn: (file: File) => uploadPolicyFile(file, "Files"),
    onSuccess: (saved) => {
      setErr("");
      setEditing(false);
      setCreating(false);
      setSelectedId(saved.id);
      qc.invalidateQueries({ queryKey: ["policies"] });
    },
    onError: (e) => setErr(e instanceof Error ? e.message : "Upload failed."),
  });

  async function download(id: string) {
    try {
      const { url } = await getPolicyDownloadUrl(id);
      window.open(url, "_blank", "noopener");
    } catch (e) {
      setErr(e instanceof Error ? e.message : "Download failed.");
    }
  }

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !editing) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, editing]);

  if (!isHr) return null;

  function selectDoc(id: string) {
    if (editing && !confirm("Discard unsaved changes?")) return;
    setSelectedId(id);
    setEditing(false);
    setCreating(false);
    setErr("");
  }
  function startCreate() {
    if (editing && !confirm("Discard unsaved changes?")) return;
    setSelectedId(null);
    setDraft({ title: "", category: "", content: "" });
    setCreating(true);
    setEditing(true);
    setErr("");
  }
  function startEdit() {
    if (!doc.data) return;
    setDraft({ title: doc.data.title, category: doc.data.category, content: doc.data.content });
    setCreating(false);
    setEditing(true);
    setErr("");
  }
  function closeDrawer() {
    if (editing && !confirm("Discard unsaved changes?")) return;
    setOpen(false);
    setEditing(false);
    setCreating(false);
    setErr("");
  }

  const isFile = doc.data?.kind === "file";
  const isSystem = doc.data?.slug === SYSTEM_SLUG;

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        title="Company handbook (OKF)"
        aria-label="Open company handbook"
        className="fixed bottom-6 right-6 z-[60] flex h-12 w-12 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition hover:opacity-90"
      >
        <NotebookPen className="h-5 w-5" />
      </button>

      {open && (
        <div className="fixed inset-0 z-[70] flex justify-end bg-black/50" onClick={closeDrawer}>
          <div
            onClick={(e) => e.stopPropagation()}
            className="flex h-full w-full max-w-5xl flex-col bg-card shadow-xl"
          >
            <div className="flex items-center justify-between border-b px-5 py-3">
              <h2 className="text-base font-semibold">Company Handbook — OKF</h2>
              <button
                type="button"
                onClick={closeDrawer}
                aria-label="Close"
                className="rounded-md p-1.5 text-muted-foreground hover:bg-secondary"
              >
                <X className="h-5 w-5" />
              </button>
            </div>

            <div className="flex min-h-0 flex-1">
              {/* Document list */}
              <div className="flex w-64 shrink-0 flex-col border-r">
                <div className="flex flex-col gap-2 border-b p-2">
                  <button
                    type="button"
                    onClick={startCreate}
                    className="flex w-full items-center justify-center gap-1.5 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground hover:opacity-90"
                  >
                    <Plus className="h-4 w-4" /> New document
                  </button>
                  <button
                    type="button"
                    onClick={() => fileInputRef.current?.click()}
                    disabled={upload.isPending}
                    className="flex w-full items-center justify-center gap-1.5 rounded-md bg-secondary px-3 py-2 text-sm font-medium hover:opacity-90 disabled:opacity-50"
                  >
                    <Upload className="h-4 w-4" /> {upload.isPending ? "Uploading…" : "Upload file"}
                  </button>
                  <input
                    ref={fileInputRef}
                    type="file"
                    className="hidden"
                    onChange={(e) => {
                      const f = e.target.files?.[0];
                      if (f) upload.mutate(f);
                      e.target.value = "";
                    }}
                  />
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto p-2">
                  {list.isLoading ? (
                    <p className="p-2 text-sm text-muted-foreground">Loading…</p>
                  ) : list.isError ? (
                    <p className="p-2 text-sm text-red-600">Could not load documents.</p>
                  ) : (
                    grouped.map(([cat, docs]) => (
                      <div key={cat} className="mb-3">
                        <p className="px-2 py-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                          {cat}
                        </p>
                        {docs.map((d) => (
                          <button
                            key={d.id}
                            type="button"
                            onClick={() => selectDoc(d.id)}
                            className={`block w-full truncate rounded-md px-2 py-1.5 text-left text-sm transition ${
                              d.id === selectedId
                                ? "bg-accent text-accent-foreground"
                                : "hover:bg-secondary"
                            }`}
                          >
                            {d.title}
                            {d.kind === "file" && (
                              <span className="ml-1 text-xs text-muted-foreground">(file)</span>
                            )}
                          </button>
                        ))}
                      </div>
                    ))
                  )}
                </div>
              </div>

              {/* Detail / editor */}
              <div className="flex min-h-0 flex-1 flex-col">
                {editing ? (
                  <div className="flex min-h-0 flex-1 flex-col gap-3 p-5">
                    <div className="flex gap-3">
                      <input
                        value={draft.title}
                        onChange={(e) => setDraft({ ...draft, title: e.target.value })}
                        placeholder="Title"
                        className="flex-1 rounded-md border border-input bg-background px-3 py-2 text-sm"
                      />
                      <input
                        value={draft.category}
                        onChange={(e) => setDraft({ ...draft, category: e.target.value })}
                        placeholder="Category"
                        className="w-56 rounded-md border border-input bg-background px-3 py-2 text-sm"
                      />
                    </div>
                    <textarea
                      value={draft.content}
                      onChange={(e) => setDraft({ ...draft, content: e.target.value })}
                      spellCheck={false}
                      placeholder="Markdown content…"
                      className="min-h-0 flex-1 w-full resize-none rounded-md border border-input bg-background p-3 font-mono text-xs leading-relaxed focus:outline-none focus:ring-2 focus:ring-ring"
                    />
                    <div className="flex items-center justify-between gap-3">
                      <p className="min-w-0 flex-1 truncate text-xs text-red-600">{err}</p>
                      <div className="flex shrink-0 gap-2">
                        <button
                          type="button"
                          onClick={() => {
                            setEditing(false);
                            setCreating(false);
                            setErr("");
                          }}
                          className="rounded-md bg-secondary px-4 py-2 text-sm font-medium hover:opacity-90"
                        >
                          Cancel
                        </button>
                        <button
                          type="button"
                          onClick={() => save.mutate()}
                          disabled={save.isPending || !draft.title.trim()}
                          className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
                        >
                          {save.isPending ? "Saving…" : "Save"}
                        </button>
                      </div>
                    </div>
                  </div>
                ) : !selectedId ? (
                  <div className="flex flex-1 items-center justify-center p-8 text-center text-sm text-muted-foreground">
                    Select a document on the left, or create a new one.
                  </div>
                ) : doc.isLoading ? (
                  <p className="p-5 text-sm text-muted-foreground">Loading…</p>
                ) : doc.isError || !doc.data ? (
                  <p className="p-5 text-sm text-red-600">Could not load this document.</p>
                ) : (
                  <>
                    <div className="flex items-start justify-between gap-3 border-b px-5 py-3">
                      <div className="min-w-0">
                        <h3 className="truncate text-sm font-semibold">{doc.data.title}</h3>
                        <p className="truncate text-xs text-muted-foreground">
                          {doc.data.category}
                          {doc.data.updated_at &&
                            ` · edited ${new Date(doc.data.updated_at).toLocaleDateString()}`}
                          {doc.data.updated_by_name && ` by ${doc.data.updated_by_name}`}
                        </p>
                      </div>
                      <div className="flex shrink-0 gap-2">
                        {!isFile && (
                          <button
                            type="button"
                            onClick={startEdit}
                            className="rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90"
                          >
                            Edit
                          </button>
                        )}
                        {!isSystem && (
                          <button
                            type="button"
                            onClick={() => {
                              if (confirm(`Delete "${doc.data.title}"? This can't be undone.`))
                                remove.mutate();
                            }}
                            disabled={remove.isPending}
                            aria-label="Delete document"
                            className="rounded-md p-1.5 text-red-600 hover:bg-red-50 disabled:opacity-50"
                          >
                            <Trash2 className="h-4 w-4" />
                          </button>
                        )}
                      </div>
                    </div>
                    <div className="min-h-0 flex-1 overflow-y-auto p-5">
                      {isFile ? (
                        <div className="space-y-3">
                          <p className="text-sm text-muted-foreground">
                            Attached file: {doc.data.file_name ?? "file"}
                            {doc.data.size_bytes
                              ? ` · ${Math.max(1, Math.round(doc.data.size_bytes / 1024))} KB`
                              : ""}
                          </p>
                          <button
                            type="button"
                            onClick={() => download(doc.data!.id)}
                            className="inline-flex items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:opacity-90"
                          >
                            <Download className="h-4 w-4" /> Download
                          </button>
                          {err && <p className="text-sm text-red-600">{err}</p>}
                        </div>
                      ) : (
                        <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-foreground">
                          {doc.data.content}
                        </pre>
                      )}
                    </div>
                  </>
                )}
              </div>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
