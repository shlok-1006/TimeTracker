import { create } from "zustand";
import { invoker, type EmployeeSession } from "@/lib/tauri";

type SessionState = {
  session: EmployeeSession | null;
  hydrated: boolean;
  /** True when the session ended because the token expired/was revoked (vs a
   *  deliberate sign-out) — the login screen shows a "please sign in again" note. */
  expired: boolean;
  /** Restore the session from the keychain via the backend (`restore_session`). */
  hydrate: () => Promise<void>;
  setSession: (s: EmployeeSession) => void;
  /** End the session because auth died server-side (revoke best-effort, flag it). */
  markExpired: () => Promise<void>;
  clear: () => Promise<void>;
};

/**
 * Desktop session (Zustand). The JWT lives in the OS keychain; this only holds
 * the lightweight session info needed to render and to pass `userId` to
 * tracking commands. Survives client-side navigation; restored on full load.
 */
export const useSession = create<SessionState>()((set) => ({
  session: null,
  hydrated: false,
  expired: false,
  hydrate: async () => {
    try {
      const invoke = await invoker();
      const restored = await invoke<EmployeeSession | null>("restore_session");
      set({ session: restored ?? null, hydrated: true });
    } catch {
      set({ hydrated: true }); // browser preview / not in Tauri
    }
  },
  setSession: (session) => set({ session, expired: false }),
  markExpired: async () => {
    try {
      const invoke = await invoker();
      await invoke("logout");
    } catch {
      /* ignore */
    }
    set({ session: null, expired: true });
  },
  clear: async () => {
    try {
      const invoke = await invoker();
      await invoke("logout");
    } catch {
      /* ignore */
    }
    set({ session: null, expired: false });
  },
}));
