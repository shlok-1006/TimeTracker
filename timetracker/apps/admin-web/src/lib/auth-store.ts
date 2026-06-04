import { create } from "zustand";
import type { Role } from "@timetracker/shared";

export type DashboardUser = {
  id: string;
  name: string;
  email: string;
  role: Role;
  team: string | null;
};

const STORAGE_KEY = "timetracker-admin-auth";

type AuthState = {
  token: string | null;
  user: DashboardUser | null;
  /** Becomes true once we've read localStorage on the client. */
  hydrated: boolean;
  setSession: (token: string, user: DashboardUser) => void;
  clear: () => void;
  hydrate: () => void;
};

/**
 * Auth state (Zustand, not Redux — per CLAUDE.md). Persistence is manual and
 * `window`-guarded so it is safe during Next.js server rendering.
 */
export const useAuthStore = create<AuthState>()((set) => ({
  token: null,
  user: null,
  hydrated: false,
  setSession: (token, user) => {
    if (typeof window !== "undefined") {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ token, user }));
    }
    set({ token, user });
  },
  clear: () => {
    if (typeof window !== "undefined") {
      localStorage.removeItem(STORAGE_KEY);
    }
    set({ token: null, user: null });
  },
  hydrate: () => {
    if (typeof window === "undefined") return;
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      try {
        const parsed = JSON.parse(raw) as { token: string; user: DashboardUser };
        set({ token: parsed.token, user: parsed.user, hydrated: true });
        return;
      } catch {
        /* fall through to clear hydrated flag */
      }
    }
    set({ hydrated: true });
  },
}));
