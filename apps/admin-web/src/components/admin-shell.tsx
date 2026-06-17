"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useAuthStore } from "@/lib/auth-store";
import { useAdminSession } from "@/components/use-admin-session";
import { cn } from "@/lib/utils";

/** Routes that render without the admin sidebar (auth / entry redirect). */
const BARE_ROUTES = new Set(["/login", "/"]);

type NavItem = { href: string; label: string; hrOnly?: boolean };

const NAV: NavItem[] = [
  { href: "/dashboard", label: "Dashboard" },
  { href: "/teams", label: "Teams" },
  { href: "/leave", label: "Leave" },
  { href: "/attendance", label: "Attendance" },
  { href: "/onboarding", label: "Onboarding", hrOnly: true },
  { href: "/manage", label: "Manage users", hrOnly: true },
];

const ROLE_LABEL: Record<string, string> = {
  hr: "HR (all employees)",
  project_manager: "Project manager",
};

/** Wraps the app: bare routes pass through; everything else gets the sidebar. */
export function AppChrome({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  if (BARE_ROUTES.has(pathname)) return <>{children}</>;
  return <AdminShell>{children}</AdminShell>;
}

function AdminShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const router = useRouter();
  const { user, ready } = useAdminSession();
  const clear = useAuthStore((s) => s.clear);

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

  const items = NAV.filter((i) => !i.hrOnly || user.role === "hr");

  return (
    <div className="flex min-h-screen">
      <aside className="sticky top-0 flex h-screen w-60 shrink-0 flex-col border-r bg-card">
        <div className="border-b px-5 py-5">
          <h1 className="text-lg font-bold tracking-tight">TimeTracker</h1>
          <p className="text-xs text-muted-foreground">Admin</p>
        </div>

        <nav className="flex flex-1 flex-col gap-1 overflow-y-auto p-3">
          {items.map((item) => {
            const active =
              pathname === item.href || pathname.startsWith(`${item.href}/`);
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
          <p className="mb-2 text-xs text-muted-foreground">
            {ROLE_LABEL[user.role] ?? user.role}
          </p>
          <button
            onClick={signOut}
            className="w-full rounded-md bg-secondary px-3 py-2 text-sm font-medium hover:opacity-90"
          >
            Sign out
          </button>
        </div>
      </aside>

      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}
