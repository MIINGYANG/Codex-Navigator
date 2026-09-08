export type ThemeMode = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";
export const THEME_KEY = "questionTrail.theme";

export function parseTheme(value: unknown): ThemeMode {
  return value === "light" || value === "dark" ? value : "system";
}

export function resolveTheme(
  mode: ThemeMode,
  systemDark: boolean,
): ResolvedTheme {
  return mode === "system" ? (systemDark ? "dark" : "light") : mode;
}

export interface ThemeEnvironment {
  read(): unknown;
  write(mode: ThemeMode): void;
  systemDark(): boolean;
  listenSystem(listener: (dark: boolean) => void): () => void;
  apply(theme: ResolvedTheme): void;
}

// Importing this module never touches browser globals; the environment is injected
// at startup so blocked storage and OS preference changes can be tested directly.
export function createThemeStore() {
  let snapshot: { mode: ThemeMode; resolved: ResolvedTheme } = {
    mode: "system",
    resolved: "light",
  };
  let environment: ThemeEnvironment | undefined;
  let systemDark = false;
  let stopListening: (() => void) | undefined;
  const listeners = new Set<() => void>();
  function update(mode: ThemeMode) {
    const resolved = resolveTheme(mode, systemDark);
    environment?.apply(resolved);
    if (snapshot.mode === mode && snapshot.resolved === resolved) return;
    snapshot = { mode, resolved };
    for (const listener of listeners) listener();
  }
  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    init(nextEnvironment: ThemeEnvironment) {
      stopListening?.();
      environment = nextEnvironment;
      let mode: ThemeMode = "system";
      try {
        mode = parseTheme(environment.read());
      } catch {
        /* Storage may be blocked. */
      }
      systemDark = environment.systemDark();
      update(mode);
      stopListening = environment.listenSystem((dark) => {
        systemDark = dark;
        if (snapshot.mode === "system") update("system");
      });
    },
    setMode(value: ThemeMode) {
      const mode = parseTheme(value);
      update(mode);
      try {
        environment?.write(mode);
      } catch {
        /* In-memory selection remains usable. */
      }
    },
    dispose() {
      stopListening?.();
      stopListening = undefined;
      environment = undefined;
      listeners.clear();
    },
  };
}

export const themeStore = createThemeStore();

export function initTheme() {
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  themeStore.init({
    read: () => window.localStorage.getItem(THEME_KEY),
    write: (mode) => window.localStorage.setItem(THEME_KEY, mode),
    systemDark: () => query.matches,
    listenSystem(listener) {
      const change = (event: MediaQueryListEvent) => listener(event.matches);
      query.addEventListener("change", change);
      return () => query.removeEventListener("change", change);
    },
    apply(theme) {
      document.documentElement.dataset.theme = theme;
      document.documentElement.style.colorScheme = theme;
    },
  });
}
