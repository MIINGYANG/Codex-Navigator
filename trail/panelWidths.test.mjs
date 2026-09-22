import assert from "node:assert/strict";
import test from "node:test";
import {
  PANEL_DEFAULTS,
  panelPreferences,
  panelResizeBounds,
  resolvePanelWidths,
} from "./panelWidths.ts";

test("面板偏好逐项校验损坏存储，有限数字夹紧到允许范围", () => {
  for (const value of [null, undefined, false, 42, "{broken", []])
    assert.deepEqual(panelPreferences(value), PANEL_DEFAULTS);
  assert.deepEqual(panelPreferences({ sidebar: 400, detail: "500" }), {
    sidebar: 400,
    detail: 360,
  });
  assert.deepEqual(
    panelPreferences({ sidebar: Infinity, detail: NaN }),
    PANEL_DEFAULTS,
  );
  assert.deepEqual(panelPreferences({ sidebar: -40, detail: 900 }), {
    sidebar: 220,
    detail: 640,
  });
  assert.deepEqual(panelPreferences({ sidebar: 300.4, detail: 500.8 }), {
    sidebar: 300,
    detail: 501,
  });
});

test("桌面收窄按可缩空间分摊，画布至少480px且首选宽度保留", () => {
  const preferences = Object.freeze({ sidebar: 420, detail: 640 });
  assert.deepEqual(resolvePanelWidths(preferences, 1848), preferences);
  const fitted = resolvePanelWidths(preferences, 1280);
  assert.deepEqual(fitted, { sidebar: 324, detail: 476 });
  assert.equal(1280 - fitted.sidebar - fitted.detail, 480);
  assert.deepEqual(resolvePanelWidths(preferences, 1848), preferences);
  assert.deepEqual(preferences, { sidebar: 420, detail: 640 });
});

test("收起会话栏释放全部预算，不丢失再次展开的首选宽度", () => {
  const preferences = { sidebar: 420, detail: 640 };
  assert.deepEqual(resolvePanelWidths(preferences, 1280, false), {
    sidebar: 0,
    detail: 640,
  });
  assert.deepEqual(resolvePanelWidths(preferences, 1848, true), preferences);
});

test("默认布局不意外扩张，达到最小尺寸的面板无需继续压缩", () => {
  assert.deepEqual(resolvePanelWidths(PANEL_DEFAULTS, 1280), PANEL_DEFAULTS);
  assert.deepEqual(resolvePanelWidths({ sidebar: 220, detail: 640 }, 1280), {
    sidebar: 220,
    detail: 580,
  });
  assert.deepEqual(resolvePanelWidths({ sidebar: 420, detail: 300 }, 1100), {
    sidebar: 320,
    detail: 300,
  });
  assert.deepEqual(resolvePanelWidths(PANEL_DEFAULTS, 390), {
    sidebar: 220,
    detail: 300,
  });
});

test("拖动上限考虑另一可见面板，保持画布预算与全局范围", () => {
  assert.deepEqual(panelResizeBounds("sidebar", 1280, 360), {
    min: 220,
    max: 420,
  });
  assert.deepEqual(panelResizeBounds("detail", 1280, 420), {
    min: 300,
    max: 380,
  });
  assert.deepEqual(panelResizeBounds("detail", 1280, 0), {
    min: 300,
    max: 640,
  });
  assert.deepEqual(panelResizeBounds("sidebar", 1848, 640), {
    min: 220,
    max: 420,
  });
  assert.deepEqual(panelResizeBounds("detail", 390, 420), {
    min: 300,
    max: 300,
  });
});

test("全部桌面宽度组合保持范围与预算，不累积舍入溢出", () => {
  for (let viewport = 1280; viewport <= 1848; viewport += 7) {
    for (let sidebar = 220; sidebar <= 420; sidebar += 13) {
      for (let detail = 300; detail <= 640; detail += 17) {
        const fitted = resolvePanelWidths({ sidebar, detail }, viewport);
        assert.ok(fitted.sidebar >= 220 && fitted.sidebar <= sidebar);
        assert.ok(fitted.detail >= 300 && fitted.detail <= detail);
        assert.ok(fitted.sidebar + fitted.detail <= viewport - 480);
      }
    }
  }
});
