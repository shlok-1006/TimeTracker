import { create } from "zustand";
import type { Role } from "@timetracker/shared";

export type PortalUser = {
  id: string;
  name: string;
  email: string;
  role: Role;
  team: string | null;
};

const STORAGE_KEY = "timetracker-employee-auth";

type Persisted = {
  token: string;
  refreshToken: string;
  user: PortalUser;
};

type AuthState = {
  token: string | null;
  refreshToken: string | null;
  user: PortalUser | null;
  /** Becomes true once we've read localStorage on the client. */
  hydrated: boolean;
  setSession: (token: string, refreshToken: string, user: PortalUser) => void;
  /** Update just the token pair (used after a refresh rotation). */
  setTokens: (token: string, refreshToken: string) => void;
  clear: () => void;
  hydrate: () => void;
};

function persist(state: { token: string | null; refreshToken: string | null; user: PortalUser | null }) {
  if (typeof window === "undefined") return;
  if (state.token && state.refreshToken && state.user) {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ token: state.token, refreshToken: state.refreshToken, user: state.user }),
    );
  } else {
    localStorage.removeItem(STORAGE_KEY);
  }
}

/**
 * Auth state (Zustand, not Redux — per CLAUDE.md). Holds the access + refresh
 * tokens; persistence is manual and `window`-guarded for SSR safety.
 */
export const useAuthStore = create<AuthState>()((set, get) => ({
  token: null,
  refreshToken: null,
  user: null,
  hydrated: false,
  setSession: (token, refreshToken, user) => {
    persist({ token, refreshToken, user });
    set({ token, refreshToken, user });
  },
  setTokens: (token, refreshToken) => {
    persist({ token, refreshToken, user: get().user });
    set({ token, refreshToken });
  },
  clear: () => {
    persist({ token: null, refreshToken: null, user: null });
    set({ token: null, refreshToken: null, user: null });
  },
  hydrate: () => {
    if (typeof window === "undefined") return;
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      try {
        const p = JSON.parse(raw) as Persisted;
        set({ token: p.token, refreshToken: p.refreshToken, user: p.user, hydrated: true });
        return;
      } catch {
        /* fall through */
      }
    }
    set({ hydrated: true });
  },
}));
