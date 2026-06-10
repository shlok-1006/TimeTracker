"use client";

import { useState, type FormEvent } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

type Ticket = {
  id: string;
  title: string;
  state: string;
  project: string | null;
  labels: string[];
  description_excerpt: string;
};
type TicketRequest = {
  id: string;
  ticket_id: string;
  ticket_title: string | null;
  status: string;
  created_at: string;
};

const REQ_BADGE: Record<string, string> = {
  pending: "bg-amber-100 text-amber-800",
  approved: "bg-green-100 text-green-800",
  rejected: "bg-red-100 text-red-800",
};

export function Tickets() {
  const qc = useQueryClient();
  const [ticketInput, setTicketInput] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const assigned = useQuery({
    queryKey: ["me_tickets"],
    queryFn: async () => (await invoker())<{ tickets: Ticket[] }>("me_tickets"),
    refetchInterval: 60000,
    retry: false,
  });
  const requests = useQuery({
    queryKey: ["my_ticket_requests"],
    queryFn: async () => (await invoker())<{ requests: TicketRequest[] }>("my_ticket_requests"),
    refetchInterval: 30000,
  });

  const request = useMutation({
    mutationFn: async (ticket: string) =>
      (await invoker())("request_ticket", { ticket }),
    onSuccess: () => {
      setMsg("Request sent — the ticket owner has been emailed to approve it.");
      setErr(null);
      setTicketInput("");
      qc.invalidateQueries({ queryKey: ["my_ticket_requests"] });
    },
    onError: (e) => {
      setErr(typeof e === "string" ? e : (e as Error).message ?? "Request failed.");
      setMsg(null);
    },
  });

  function submit(e: FormEvent) {
    e.preventDefault();
    if (ticketInput.trim()) request.mutate(ticketInput.trim());
  }

  const tickets = assigned.data?.tickets ?? [];
  const reqs = requests.data?.requests ?? [];

  return (
    <section className="flex flex-col gap-4 rounded-lg border border-slate-200 p-6 dark:border-slate-800">
      <h3 className="font-semibold">My tickets</h3>

      {/* Assigned tickets / empty state */}
      {assigned.isLoading ? (
        <p className="text-sm text-slate-500">Loading…</p>
      ) : tickets.length > 0 ? (
        <ul className="flex flex-col gap-2">
          {tickets.map((t) => (
            <li key={t.id} className="rounded-md border border-slate-200 p-3 dark:border-slate-700">
              <div className="flex items-center justify-between">
                <span className="font-medium">{t.title}</span>
                <span className="text-xs text-slate-500">{t.state}</span>
              </div>
              <div className="mt-1 flex flex-wrap gap-1 text-xs text-slate-500">
                {t.project && <span>{t.project}</span>}
                {t.labels.map((l) => (
                  <span key={l} className="rounded bg-slate-100 px-1.5 dark:bg-slate-800">
                    {l}
                  </span>
                ))}
              </div>
              {t.description_excerpt && (
                <p className="mt-1 text-xs text-slate-500">{t.description_excerpt}</p>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <p className="rounded-md bg-slate-50 p-3 text-sm text-slate-600 dark:bg-slate-800/50 dark:text-slate-300">
          No tickets have been assigned to you.
        </p>
      )}

      {/* Manual entry */}
      <form onSubmit={submit} className="flex flex-col gap-2 border-t border-slate-200 pt-4 dark:border-slate-800">
        <label className="text-sm font-medium">Request access to a ticket</label>
        <div className="flex gap-2">
          <input
            value={ticketInput}
            onChange={(e) => setTicketInput(e.target.value)}
            placeholder="Ticket ID (e.g. ENG-123)"
            className="flex-1 rounded-md border border-slate-300 px-3 py-2 text-sm dark:border-slate-700 dark:bg-slate-900"
          />
          <button
            type="submit"
            disabled={request.isPending}
            className="rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
          >
            {request.isPending ? "Sending…" : "Request"}
          </button>
        </div>
        {msg && <p className="text-sm text-green-600">{msg}</p>}
        {err && <p className="text-sm text-red-600">{err}</p>}
      </form>

      {/* Requests + statuses */}
      {reqs.length > 0 && (
        <div className="flex flex-col gap-2 border-t border-slate-200 pt-4 dark:border-slate-800">
          <p className="text-sm font-medium">Your requests</p>
          {reqs.map((r) => (
            <div key={r.id} className="flex items-center justify-between text-sm">
              <span>{r.ticket_title ?? r.ticket_id}</span>
              <span className={`rounded-full px-2 py-0.5 text-xs ${REQ_BADGE[r.status] ?? "bg-slate-100 text-slate-700"}`}>
                {r.status}
              </span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
