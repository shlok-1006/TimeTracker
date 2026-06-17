"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createCandidate,
  fetchCandidates,
  fetchStages,
  updateCandidate,
  type Candidate,
} from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";

const STATUS_BADGE: Record<string, string> = {
  active: "bg-secondary text-secondary-foreground",
  hired: "bg-green-100 text-green-800",
  rejected: "bg-red-100 text-red-800",
};

function CandidateCard({
  candidate,
  stageIds,
  onMove,
  moving,
}: {
  candidate: Candidate;
  stageIds: { id: string; name: string }[];
  onMove: (stageId: string) => void;
  moving: boolean;
}) {
  return (
    <div className="rounded-lg border bg-card p-3 text-card-foreground shadow-sm">
      <Link href={`/onboarding/${candidate.id}`} className="block hover:underline">
        <div className="font-medium">{candidate.name}</div>
        <div className="truncate text-xs text-muted-foreground">{candidate.email}</div>
        {candidate.position && (
          <div className="mt-0.5 text-xs text-muted-foreground">{candidate.position}</div>
        )}
      </Link>
      <div className="mt-2 flex items-center justify-between gap-2">
        <span
          className={`rounded-full px-2 py-0.5 text-[10px] font-medium ${
            STATUS_BADGE[candidate.status] ?? "bg-secondary"
          }`}
        >
          {candidate.status}
        </span>
        <select
          aria-label="Move to stage"
          value={candidate.stage_id}
          disabled={moving || candidate.status === "hired"}
          onChange={(e) => onMove(e.target.value)}
          className="rounded-md border border-input bg-background px-1.5 py-1 text-xs disabled:opacity-50"
        >
          {stageIds.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}

export default function OnboardingPage() {
  const { ready } = useAdminSession();
  const qc = useQueryClient();

  const stages = useQuery({ queryKey: ["onboarding_stages"], queryFn: fetchStages, enabled: ready });
  const candidates = useQuery({
    queryKey: ["candidates"],
    queryFn: fetchCandidates,
    enabled: ready,
  });

  const move = useMutation({
    mutationFn: ({ id, stageId }: { id: string; stageId: string }) =>
      updateCandidate(id, { stage_id: stageId }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["candidates"] }),
  });

  const [form, setForm] = useState({ name: "", email: "", position: "" });
  const create = useMutation({
    mutationFn: () =>
      createCandidate({
        name: form.name.trim(),
        email: form.email.trim(),
        position: form.position.trim(),
      }),
    onSuccess: () => {
      setForm({ name: "", email: "", position: "" });
      qc.invalidateQueries({ queryKey: ["candidates"] });
    },
  });

  const byStage = useMemo(() => {
    const map = new Map<string, Candidate[]>();
    for (const c of candidates.data ?? []) {
      const arr = map.get(c.stage_id) ?? [];
      arr.push(c);
      map.set(c.stage_id, arr);
    }
    return map;
  }, [candidates.data]);

  const stageList = (stages.data ?? []).map((s) => ({ id: s.id, name: s.name }));

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <main className="container mx-auto flex max-w-6xl flex-col gap-6 py-12">
      <header>
        <h1 className="text-2xl font-bold tracking-tight">Onboarding</h1>
      </header>

      <section className="rounded-lg border bg-card p-4 text-card-foreground">
        <h2 className="mb-3 text-sm font-semibold">Add candidate</h2>
        <form
          className="flex flex-wrap items-end gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (form.name.trim() && form.email.includes("@")) create.mutate();
          }}
        >
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Name</span>
            <input
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
              required
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Email</span>
            <input
              type="email"
              value={form.email}
              onChange={(e) => setForm({ ...form, email: e.target.value })}
              className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
              required
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Position</span>
            <input
              value={form.position}
              onChange={(e) => setForm({ ...form, position: e.target.value })}
              className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
            />
          </label>
          <button
            type="submit"
            disabled={create.isPending}
            className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
          >
            {create.isPending ? "Adding…" : "Add"}
          </button>
        </form>
        {create.isError && (
          <p className="mt-2 text-sm text-red-600">{(create.error as Error).message}</p>
        )}
      </section>

      {(stages.error || candidates.error) && (
        <p className="text-red-600">
          {((stages.error || candidates.error) as Error).message}
        </p>
      )}

      <div className="grid auto-cols-[minmax(220px,1fr)] grid-flow-col gap-4 overflow-x-auto pb-4">
        {stages.data?.map((stage) => {
          const cards = byStage.get(stage.id) ?? [];
          return (
            <div key={stage.id} className="flex min-w-[220px] flex-col gap-3">
              <div className="flex items-center justify-between">
                <h3 className="text-sm font-semibold">{stage.name}</h3>
                <span className="text-xs text-muted-foreground">{cards.length}</span>
              </div>
              <div className="flex flex-col gap-3">
                {cards.map((c) => (
                  <CandidateCard
                    key={c.id}
                    candidate={c}
                    stageIds={stageList}
                    moving={move.isPending}
                    onMove={(stageId) => move.mutate({ id: c.id, stageId })}
                  />
                ))}
                {cards.length === 0 && (
                  <p className="rounded-lg border border-dashed p-3 text-center text-xs text-muted-foreground">
                    empty
                  </p>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </main>
  );
}
