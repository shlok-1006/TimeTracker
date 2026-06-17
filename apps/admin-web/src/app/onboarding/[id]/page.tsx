"use client";

import { useRef, useState } from "react";
import Link from "next/link";
import { useParams, useRouter } from "next/navigation";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  addCandidateTask,
  convertCandidate,
  deleteCandidate,
  deleteCandidateTask,
  fetchCandidate,
  fetchStages,
  toggleCandidateTask,
  updateCandidate,
  uploadCandidateDocument,
} from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";

export default function CandidateDetailPage() {
  const { id } = useParams<{ id: string }>();
  const { ready } = useAdminSession();
  const router = useRouter();
  const qc = useQueryClient();
  const key = ["candidate", id];

  const detail = useQuery({
    queryKey: key,
    queryFn: () => fetchCandidate(id),
    enabled: ready && !!id,
  });
  const stages = useQuery({ queryKey: ["onboarding_stages"], queryFn: fetchStages, enabled: ready });

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: key });
    qc.invalidateQueries({ queryKey: ["candidates"] });
  };

  const setStage = useMutation({
    mutationFn: (stageId: string) => updateCandidate(id, { stage_id: stageId }),
    onSuccess: invalidate,
  });
  const setStatus = useMutation({
    mutationFn: (status: string) => updateCandidate(id, { status }),
    onSuccess: invalidate,
  });

  const [taskTitle, setTaskTitle] = useState("");
  const addTask = useMutation({
    mutationFn: () => addCandidateTask(id, taskTitle.trim()),
    onSuccess: () => {
      setTaskTitle("");
      invalidate();
    },
  });
  const toggleTask = useMutation({
    mutationFn: ({ tid, done }: { tid: string; done: boolean }) => toggleCandidateTask(tid, done),
    onSuccess: invalidate,
  });
  const removeTask = useMutation({
    mutationFn: (tid: string) => deleteCandidateTask(tid),
    onSuccess: invalidate,
  });

  const fileRef = useRef<HTMLInputElement>(null);
  const [docType, setDocType] = useState("resume");
  const upload = useMutation({
    mutationFn: (file: File) => uploadCandidateDocument(id, file, docType.trim() || "document"),
    onSuccess: () => {
      if (fileRef.current) fileRef.current.value = "";
      invalidate();
    },
  });

  const convert = useMutation({
    mutationFn: () => convertCandidate(id),
    onSuccess: invalidate,
  });
  const remove = useMutation({
    mutationFn: () => deleteCandidate(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["candidates"] });
      router.replace("/onboarding");
    },
  });

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  const c = detail.data?.candidate;

  return (
    <main className="container mx-auto flex max-w-3xl flex-col gap-6 py-12">
      <header className="flex items-center justify-between">
        <h1 className="text-2xl font-bold tracking-tight">Candidate</h1>
        <Link
          href="/onboarding"
          className="rounded-md bg-secondary px-4 py-2 text-sm font-medium hover:opacity-90"
        >
          ← Back to board
        </Link>
      </header>

      {detail.isLoading && <p className="text-muted-foreground">Loading…</p>}
      {detail.error && <p className="text-red-600">{(detail.error as Error).message}</p>}

      {c && (
        <>
          <section className="rounded-lg border bg-card p-6 text-card-foreground">
            <div className="flex items-start justify-between gap-4">
              <div>
                <h2 className="text-xl font-semibold">{c.name}</h2>
                <p className="text-sm text-muted-foreground">{c.email}</p>
                {c.position && <p className="text-sm text-muted-foreground">{c.position}</p>}
              </div>
              <span className="rounded-full bg-secondary px-3 py-1 text-xs font-medium">
                {c.status}
              </span>
            </div>

            <div className="mt-4 flex flex-wrap items-center gap-4">
              <label className="flex items-center gap-2 text-sm">
                <span className="text-muted-foreground">Stage</span>
                <select
                  value={c.stage_id}
                  disabled={setStage.isPending || c.status === "hired"}
                  onChange={(e) => setStage.mutate(e.target.value)}
                  className="rounded-md border border-input bg-background px-2 py-1.5 text-sm disabled:opacity-50"
                >
                  {stages.data?.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.name}
                    </option>
                  ))}
                </select>
              </label>
              {c.status !== "hired" && (
                <button
                  onClick={() => setStatus.mutate(c.status === "rejected" ? "active" : "rejected")}
                  disabled={setStatus.isPending}
                  className="rounded-md bg-secondary px-3 py-1.5 text-sm font-medium hover:opacity-90 disabled:opacity-50"
                >
                  {c.status === "rejected" ? "Reactivate" : "Reject"}
                </button>
              )}
            </div>
          </section>

          {/* Checklist */}
          <section className="rounded-lg border bg-card p-6 text-card-foreground">
            <h2 className="mb-3 text-lg font-semibold">Checklist</h2>
            <ul className="flex flex-col gap-2">
              {detail.data?.tasks.map((t) => (
                <li key={t.id} className="flex items-center gap-3">
                  <input
                    type="checkbox"
                    checked={t.done}
                    onChange={(e) => toggleTask.mutate({ tid: t.id, done: e.target.checked })}
                    className="h-4 w-4"
                  />
                  <span className={t.done ? "flex-1 text-muted-foreground line-through" : "flex-1"}>
                    {t.title}
                  </span>
                  <button
                    onClick={() => removeTask.mutate(t.id)}
                    className="text-xs text-red-600 hover:underline"
                  >
                    remove
                  </button>
                </li>
              ))}
              {detail.data?.tasks.length === 0 && (
                <li className="text-sm text-muted-foreground">No tasks yet.</li>
              )}
            </ul>
            <form
              className="mt-3 flex gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                if (taskTitle.trim()) addTask.mutate();
              }}
            >
              <input
                value={taskTitle}
                onChange={(e) => setTaskTitle(e.target.value)}
                placeholder="Add a task…"
                className="flex-1 rounded-md border border-input bg-background px-3 py-1.5 text-sm"
              />
              <button
                type="submit"
                disabled={addTask.isPending}
                className="rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
              >
                Add
              </button>
            </form>
          </section>

          {/* Documents */}
          <section className="rounded-lg border bg-card p-6 text-card-foreground">
            <h2 className="mb-3 text-lg font-semibold">Documents</h2>
            <ul className="flex flex-col gap-2">
              {detail.data?.documents.map((d) => (
                <li key={d.id} className="flex items-center justify-between gap-3 text-sm">
                  <a
                    href={d.url}
                    target="_blank"
                    rel="noreferrer"
                    className="truncate text-primary hover:underline"
                  >
                    {d.doc_type || "document"} — {d.storage_key.split("/").pop()}
                  </a>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {new Date(d.created_at).toLocaleDateString()}
                  </span>
                </li>
              ))}
              {detail.data?.documents.length === 0 && (
                <li className="text-sm text-muted-foreground">No documents yet.</li>
              )}
            </ul>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <input
                value={docType}
                onChange={(e) => setDocType(e.target.value)}
                placeholder="Type (e.g. resume)"
                className="w-40 rounded-md border border-input bg-background px-3 py-1.5 text-sm"
              />
              <input
                ref={fileRef}
                type="file"
                onChange={(e) => {
                  const f = e.target.files?.[0];
                  if (f) upload.mutate(f);
                }}
                className="text-sm"
              />
              {upload.isPending && <span className="text-xs text-muted-foreground">Uploading…</span>}
            </div>
            {upload.isError && (
              <p className="mt-2 text-sm text-red-600">{(upload.error as Error).message}</p>
            )}
          </section>

          {/* Convert / delete */}
          <section className="rounded-lg border bg-card p-6 text-card-foreground">
            <h2 className="mb-3 text-lg font-semibold">Actions</h2>
            {c.converted_user_id ? (
              <p className="text-sm text-green-700">
                Converted to an employee account.
              </p>
            ) : (
              <button
                onClick={() => convert.mutate()}
                disabled={convert.isPending}
                className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
              >
                {convert.isPending ? "Converting…" : "Convert to employee"}
              </button>
            )}
            {convert.isError && (
              <p className="mt-2 text-sm text-red-600">{(convert.error as Error).message}</p>
            )}
            {convert.isSuccess && (
              <div className="mt-3 rounded-md border border-green-300 bg-green-50 p-3 text-sm text-green-800">
                <p className="font-medium">Employee account created.</p>
                <p>
                  Temporary password (hand over once):{" "}
                  <code className="rounded bg-white px-1 py-0.5 font-mono">
                    {convert.data.password}
                  </code>
                </p>
              </div>
            )}

            <div className="mt-4 border-t pt-4">
              <button
                onClick={() => {
                  if (confirm("Delete this candidate? This cannot be undone.")) remove.mutate();
                }}
                disabled={remove.isPending}
                className="rounded-md bg-red-600 px-4 py-2 text-sm font-medium text-white hover:opacity-90 disabled:opacity-50"
              >
                Delete candidate
              </button>
            </div>
          </section>
        </>
      )}
    </main>
  );
}
