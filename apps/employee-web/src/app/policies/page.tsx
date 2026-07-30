"use client";

import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  fetchPolicies,
  fetchPolicy,
  fetchPolicyDownloadUrl,
  type PolicySummary,
} from "@/lib/api";
import { useEmployeeSession } from "@/components/use-employee-session";

/** Read-only company handbook: policy documents grouped by category. */
export default function PoliciesPage() {
  const { ready } = useEmployeeSession();
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const list = useQuery({ queryKey: ["policies"], queryFn: fetchPolicies, enabled: ready });
  const doc = useQuery({
    queryKey: ["policy", selectedId],
    queryFn: () => fetchPolicy(selectedId as string),
    enabled: ready && !!selectedId,
  });

  const grouped = useMemo(() => {
    const g: Record<string, PolicySummary[]> = {};
    for (const d of list.data ?? []) (g[d.category] ??= []).push(d);
    return Object.entries(g);
  }, [list.data]);

  // Open the first document by default.
  useEffect(() => {
    if (!selectedId && list.data && list.data.length > 0) setSelectedId(list.data[0].id);
  }, [list.data, selectedId]);

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <main className="container mx-auto flex max-w-5xl flex-col gap-6 py-8 sm:py-12">
      <div>
        <h1 className="text-2xl font-semibold">Policies &amp; Handbook</h1>
        <p className="text-sm text-muted-foreground">Company policies and guidelines.</p>
      </div>

      <div className="flex flex-col gap-4 md:flex-row">
        <aside className="shrink-0 rounded-lg border bg-card p-2 text-card-foreground md:w-64">
          {list.isLoading ? (
            <p className="p-2 text-sm text-muted-foreground">Loading…</p>
          ) : list.isError ? (
            <p className="p-2 text-sm text-red-600">Could not load policies.</p>
          ) : (list.data ?? []).length === 0 ? (
            <p className="p-2 text-sm text-muted-foreground">No policies yet.</p>
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
                    onClick={() => setSelectedId(d.id)}
                    className={`block w-full truncate rounded-md px-2 py-1.5 text-left text-sm transition ${
                      d.id === selectedId
                        ? "bg-accent text-accent-foreground"
                        : "hover:bg-secondary"
                    }`}
                  >
                    {d.title}
                  </button>
                ))}
              </div>
            ))
          )}
        </aside>

        <section className="min-w-0 flex-1 rounded-lg border bg-card p-6 text-card-foreground">
          {!selectedId ? (
            <p className="text-sm text-muted-foreground">Select a policy to read.</p>
          ) : doc.isLoading ? (
            <p className="text-sm text-muted-foreground">Loading…</p>
          ) : doc.isError || !doc.data ? (
            <p className="text-sm text-red-600">Could not load this policy.</p>
          ) : doc.data.kind === "file" ? (
            <div className="space-y-3">
              <h2 className="text-lg font-semibold">{doc.data.title}</h2>
              <p className="text-sm text-muted-foreground">
                Attached file: {doc.data.file_name ?? "file"}
              </p>
              <button
                type="button"
                onClick={async () => {
                  const { url } = await fetchPolicyDownloadUrl(doc.data!.id);
                  window.open(url, "_blank", "noopener");
                }}
                className="inline-flex items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:opacity-90"
              >
                Download
              </button>
            </div>
          ) : (
            <>
              <h2 className="mb-1 text-lg font-semibold">{doc.data.title}</h2>
              <p className="mb-4 text-xs text-muted-foreground">{doc.data.category}</p>
              <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-foreground">
                {doc.data.content}
              </pre>
            </>
          )}
        </section>
      </div>
    </main>
  );
}
