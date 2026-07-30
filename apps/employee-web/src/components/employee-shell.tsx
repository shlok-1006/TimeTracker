"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { Menu, X } from "lucide-react";
import { useAuthStore } from "@/lib/auth-store";
import { useEmployeeSession } from "@/components/use-employee-session";
import { cn } from "@/lib/utils";

/** Routes that render without the portal sidebar (auth / entry redirect). */
const BARE_ROUTES = new Set(["/login", "/"]);

const NAV = [
  { href: "/dashboard", label: "Dashboard" },
  { href: "/day", label: "My Day" },
  { href: "/leave", label: "Leave" },
  { href: "/attendance", label: "Attendance" },
  { href: "/performance", label: "My Performance" },
  { href: "/policies", label: "Policies" },
];

/** Wraps the app: bare routes pass through; everything else gets the sidebar. */
export function AppChrome({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  if (BARE_ROUTES.has(pathname)) return <>{children}</>;
  return <EmployeeShell>{children}</EmployeeShell>;
}

function EmployeeShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const router = useRouter();
  const { user, ready } = useEmployeeSession();
  const clear = useAuthStore((s) => s.clear);
  const [open, setOpen] = useState(false);

  // Close the mobile drawer whenever the route changes.
  useEffect(() => {
    setOpen(false);
  }, [pathname]);

  // Close on Escape for keyboard users.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  function signOut() {
    clear();
    router.replace("/login");
  }

  if (!ready || !user) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <div className="flex min-h-screen">
      {/* Backdrop (mobile only, when drawer open) */}
      {open && (
        <div
          aria-hidden
          onClick={() => setOpen(false)}
          className="fixed inset-0 z-40 bg-black/50 lg:hidden"
        />
      )}

      {/* Sidebar: off-canvas drawer on mobile, static column on desktop */}
      <aside
        className={cn(
          "fixed inset-y-0 left-0 z-50 flex h-screen w-64 shrink-0 flex-col border-r bg-card transition-transform duration-200 ease-out lg:sticky lg:top-0 lg:z-auto lg:w-60 lg:translate-x-0",
          open ? "translate-x-0" : "-translate-x-full",
        )}
      >
        <div className="flex items-center justify-between border-b px-5 py-5">
          <div>
            <h1 className="text-lg font-bold tracking-tight">TimeTracker</h1>
            <p className="text-xs text-muted-foreground">Employee portal</p>
          </div>
          <button
            aria-label="Close menu"
            onClick={() => setOpen(false)}
            className="rounded-md p-1 text-muted-foreground hover:bg-secondary lg:hidden"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <nav className="flex flex-1 flex-col gap-1 overflow-y-auto p-3">
          {NAV.map((item) => {
            const active = pathname === item.href || pathname.startsWith(`${item.href}/`);
            return (
              <Link
                key={item.href}
                href={item.href}
                className={cn(
                  "rounded-md px-3 py-2 text-sm font-medium transition",
                  active
                    ? "bg-accent text-accent-foreground"
                    : "text-foreground hover:bg-secondary",
                )}
              >
                {item.label}
              </Link>
            );
          })}
        </nav>

        <div className="border-t p-3">
          <p className="truncate text-sm font-medium">{user.name}</p>
          <p className="mb-2 truncate text-xs text-muted-foreground">{user.email}</p>
          <button
            onClick={signOut}
            className="w-full rounded-md bg-secondary px-3 py-2 text-sm font-medium hover:opacity-90"
          >
            Sign out
          </button>
        </div>
      </aside>

      {/* Content column with a mobile top bar */}
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="sticky top-0 z-30 flex h-14 items-center gap-3 border-b bg-card/95 px-4 backdrop-blur lg:hidden">
          <button
            aria-label="Open menu"
            onClick={() => setOpen(true)}
            className="rounded-md p-1.5 text-foreground hover:bg-secondary"
          >
            <Menu className="h-5 w-5" />
          </button>
          <span className="text-base font-bold tracking-tight">TimeTracker</span>
          <span className="text-xs text-muted-foreground">Employee</span>
        </header>

        <div className="min-w-0 flex-1">{children}</div>
      </div>
    </div>
  );
}
