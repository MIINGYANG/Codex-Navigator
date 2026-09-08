import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import {
  createThemeStore,
  initTheme,
  parseTheme,
  resolveTheme,
  THEME_KEY,
  themeStore,
} from "./theme.ts";

function fixture({ preference = null, dark = false, blocked = false } = {}) {
  const store = createThemeStore();
  const applied = [];
  const written = [];
  let listener;
  let detached = 0;
  const environment = {
    read() {
      if (blocked) throw Error("blocked");
      return preference;
    },
    write(value) {
      if (blocked) throw Error("blocked");
      written.push(value);
    },
    systemDark: () => dark,
    listenSystem(callback) {
      listener = callback;
      return () => {
        detached++;
      };
    },
    apply: (value) => applied.push(value),
  };
  store.init(environment);
  return {
    store,
    applied,
    written,
    environment,
    change: (dark) => listener(dark),
    detached: () => detached,
  };
}

test("主题输入只接受三态，默认安全跟随系统", () => {
  assert.equal(THEME_KEY, "questionTrail.theme");
  for (const value of [null, undefined, "invalid", "system", {}, 0])
    assert.equal(parseTheme(value), "system");
  assert.equal(parseTheme("dark"), "dark");
  assert.equal(parseTheme("light"), "light");
  assert.equal(resolveTheme("system", true), "dark");
  assert.equal(resolveTheme("system", false), "light");
  assert.equal(resolveTheme("light", true), "light");
});
test("初始化在订阅和首次渲染前应用保存的选择", () => {
  const f = fixture({ preference: "dark" });
  assert.deepEqual(f.store.getSnapshot(), { mode: "dark", resolved: "dark" });
  assert.deepEqual(f.applied, ["dark"]);
  assert.deepEqual(f.written, []);
});
test("跟随系统实时变化，显式选择不会被系统覆盖，恢复自动采用最新偏好", () => {
  const f = fixture({ dark: true });
  assert.equal(f.store.getSnapshot().resolved, "dark");
  f.change(false);
  assert.equal(f.store.getSnapshot().resolved, "light");
  f.store.setMode("dark");
  f.change(true);
  f.change(false);
  assert.deepEqual(f.store.getSnapshot(), { mode: "dark", resolved: "dark" });
  f.store.setMode("system");
  assert.deepEqual(f.store.getSnapshot(), {
    mode: "system",
    resolved: "light",
  });
  assert.deepEqual(f.written, ["dark", "system"]);
});
test("存储读写异常不阻止系统初始化或手动切换", () => {
  const f = fixture({ dark: true, blocked: true });
  assert.equal(f.store.getSnapshot().resolved, "dark");
  assert.doesNotThrow(() => f.store.setMode("light"));
  assert.equal(f.store.getSnapshot().resolved, "light");
});
test("稳定 snapshot 和订阅避免 React 循环更新，重新初始化清理监听", () => {
  const f = fixture();
  let changes = 0;
  const off = f.store.subscribe(() => changes++);
  const before = f.store.getSnapshot();
  f.change(false);
  assert.equal(f.store.getSnapshot(), before);
  assert.equal(changes, 0);
  f.store.setMode("dark");
  assert.equal(changes, 1);
  off();
  f.store.setMode("light");
  assert.equal(changes, 1);
  f.store.init(f.environment);
  assert.equal(f.detached(), 1);
  f.store.dispose();
  assert.equal(f.detached(), 2);
});

test("阻塞型启动脚本在应用加载前与主题解析保持一致，存储异常不闪白", () => {
  const script = readFileSync(
    new URL("./public/theme-init.js", import.meta.url),
    "utf8",
  );
  for (const stored of [null, "system", "dark", "light", "invalid"]) {
    for (const dark of [false, true]) {
      for (const blocked of [false, true]) {
        const html = { dataset: {}, style: {} };
        runInNewContext(script, {
          document: { documentElement: html },
          window: {
            localStorage: {
              getItem(key) {
                assert.equal(key, THEME_KEY);
                if (blocked) throw Error("blocked");
                return stored;
              },
            },
            matchMedia(query) {
              assert.equal(query, "(prefers-color-scheme: dark)");
              return { matches: dark };
            },
          },
        });
        const expected = resolveTheme(
          parseTheme(blocked ? null : stored),
          dark,
        );
        assert.equal(html.dataset.theme, expected);
        assert.equal(html.style.colorScheme, expected);
      }
    }
  }
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  assert.ok(
    html.indexOf('src="/theme-init.js"') < html.indexOf('type="module"'),
  );
  assert.match(html, /<script src="\/theme-init\.js"><\/script>/);
});

test("浏览器初始化同步设置 DOM 与 color-scheme，随后系统变更只影响自动模式", () => {
  const previousWindow = globalThis.window;
  const previousDocument = globalThis.document;
  const html = { dataset: {}, style: {} };
  let listener;
  const values = new Map([[THEME_KEY, "dark"]]);
  globalThis.document = { documentElement: html };
  globalThis.window = {
    localStorage: {
      getItem: (key) => values.get(key),
      setItem: (key, value) => values.set(key, value),
    },
    matchMedia: () => ({
      matches: false,
      addEventListener: (_name, callback) => {
        listener = callback;
      },
      removeEventListener() {},
    }),
  };
  try {
    initTheme();
    assert.equal(html.dataset.theme, "dark");
    assert.equal(html.style.colorScheme, "dark");
    listener({ matches: false });
    assert.equal(html.dataset.theme, "dark");
    themeStore.setMode("system");
    assert.equal(values.get(THEME_KEY), "system");
    assert.equal(html.dataset.theme, "light");
    listener({ matches: true });
    assert.equal(html.dataset.theme, "dark");
    assert.equal(html.style.colorScheme, "dark");
  } finally {
    themeStore.dispose();
    globalThis.window = previousWindow;
    globalThis.document = previousDocument;
  }
});
