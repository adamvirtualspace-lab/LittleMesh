// Light/dark mode and shadcn-style colour themes. The choice is a per-machine
// convenience, so it lives in localStorage and falls back to the defaults
// whenever storage is unavailable.

import { getCurrentWindow } from "@tauri-apps/api/window";

const MODES = ["light", "dark", "system"] as const;
const COLORS = ["neutral", "blue", "green", "orange", "rose", "violet"] as const;
type Mode = (typeof MODES)[number];
type Color = (typeof COLORS)[number];

interface Theme {
  mode: Mode;
  color: Color;
}

const KEY = "littlemesh.theme";
const DEFAULT: Theme = { mode: "system", color: "neutral" };
const systemDark = window.matchMedia("(prefers-color-scheme: dark)");

let theme = load();
let onChange: ((dark: boolean) => void) | null = null;

function load(): Theme {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    return {
      mode: MODES.includes(saved.mode) ? saved.mode : DEFAULT.mode,
      color: COLORS.includes(saved.color) ? saved.color : DEFAULT.color,
    };
  } catch {
    return { ...DEFAULT };
  }
}

export function isDark(): boolean {
  return theme.mode === "dark" || (theme.mode === "system" && systemDark.matches);
}

function apply() {
  const root = document.documentElement;
  const dark = isDark();
  root.classList.toggle("dark", dark);
  if (theme.color === "neutral") {
    delete root.dataset.theme;
  } else {
    root.dataset.theme = theme.color;
  }
  // The native title bar follows too; null means "follow the system".
  try {
    getCurrentWindow()
      .setTheme(theme.mode === "system" ? null : theme.mode)
      .catch(() => {});
  } catch {
    // Outside Tauri (a plain browser preview) there is no window to theme.
  }
  document.querySelectorAll<HTMLElement>("[data-mode]").forEach((b) => {
    b.setAttribute("aria-pressed", String(b.dataset.mode === theme.mode));
  });
  document.querySelectorAll<HTMLButtonElement>("button[data-color]").forEach((b) => {
    b.setAttribute("aria-pressed", String(b.dataset.color === theme.color));
  });
  onChange?.(dark);
}

function set(next: Partial<Theme>) {
  theme = { ...theme, ...next };
  try {
    localStorage.setItem(KEY, JSON.stringify(theme));
  } catch {
    // Not remembered this time; the theme still applies.
  }
  apply();
}

/** Calls `f` now and whenever light/dark changes. */
export function onThemeChange(f: (dark: boolean) => void) {
  onChange = f;
  f(isDark());
}

/** Wires the header button to the popover with the mode and colour choices. */
export function initThemePicker(button: HTMLElement, popover: HTMLElement) {
  const show = (open: boolean) => {
    popover.hidden = !open;
    button.setAttribute("aria-expanded", String(open));
  };
  button.addEventListener("click", (e) => {
    e.stopPropagation();
    show(popover.hidden !== false);
  });
  popover.addEventListener("click", (e) => {
    const target = (e.target as HTMLElement).closest<HTMLElement>("button");
    const mode = target?.dataset.mode as Mode | undefined;
    const color = target?.dataset.color as Color | undefined;
    if (mode) {
      set({ mode });
    } else if (color) {
      set({ color });
    }
  });
  document.addEventListener("click", (e) => {
    if (!popover.hidden && !popover.contains(e.target as Node)) {
      show(false);
    }
  });
  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !popover.hidden) {
      show(false);
      button.focus();
    }
  });
}

systemDark.addEventListener("change", () => {
  if (theme.mode === "system") {
    apply();
  }
});
apply();
