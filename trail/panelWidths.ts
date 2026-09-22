export type PanelSide = "sidebar" | "detail";
export type PanelWidths = Record<PanelSide, number>;

export const PANEL_DESKTOP_MIN = 1280;
export const PANEL_CANVAS_MIN = 480;
export const PANEL_DEFAULTS: PanelWidths = { sidebar: 250, detail: 360 };
export const PANEL_LIMITS = {
  sidebar: { min: 220, max: 420 },
  detail: { min: 300, max: 640 },
} as const;

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

export function panelPreferences(value: unknown): PanelWidths {
  const source =
    value && typeof value === "object"
      ? (value as Partial<Record<PanelSide, unknown>>)
      : {};
  function width(side: PanelSide) {
    const candidate = source[side];
    return typeof candidate === "number" && Number.isFinite(candidate)
      ? Math.round(
          clamp(candidate, PANEL_LIMITS[side].min, PANEL_LIMITS[side].max),
        )
      : PANEL_DEFAULTS[side];
  }
  return { sidebar: width("sidebar"), detail: width("detail") };
}

function panelBudget(viewportWidth: number) {
  return (
    (Number.isFinite(viewportWidth) ? viewportWidth : PANEL_DESKTOP_MIN) -
    PANEL_CANVAS_MIN
  );
}

// Derive displayed widths without overwriting the user's wider-screen preference.
// Below the desktop breakpoint, the caller switches to drawer/sheet layouts.
export function resolvePanelWidths(
  preferences: PanelWidths,
  viewportWidth: number,
  sidebarVisible = true,
): PanelWidths {
  const widths = panelPreferences(preferences);
  if (!sidebarVisible) {
    return {
      sidebar: 0,
      detail: Math.max(
        PANEL_LIMITS.detail.min,
        Math.min(widths.detail, panelBudget(viewportWidth)),
      ),
    };
  }
  const minimum = PANEL_LIMITS.sidebar.min + PANEL_LIMITS.detail.min;
  const available = Math.max(minimum, Math.floor(panelBudget(viewportWidth)));
  const total = widths.sidebar + widths.detail;
  if (total <= available) return widths;
  const flexibility = total - minimum;
  const sidebar =
    PANEL_LIMITS.sidebar.min +
    Math.round(
      ((widths.sidebar - PANEL_LIMITS.sidebar.min) * (available - minimum)) /
        flexibility,
    );
  return { sidebar, detail: available - sidebar };
}

export function panelResizeBounds(
  side: PanelSide,
  viewportWidth: number,
  otherVisibleWidth: number,
): { min: number; max: number } {
  const { min, max } = PANEL_LIMITS[side];
  const other = Number.isFinite(otherVisibleWidth)
    ? Math.max(0, otherVisibleWidth)
    : PANEL_DEFAULTS[side === "sidebar" ? "detail" : "sidebar"];
  return {
    min,
    max: Math.max(
      min,
      Math.min(max, Math.floor(panelBudget(viewportWidth) - other)),
    ),
  };
}
