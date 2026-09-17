// 仅在隔离合成 CODEX_HOME 与自建 Chrome 窗口中验收画布与整理功能。
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { once } from "node:events";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";

const execute = promisify(execFile);
const binary = path.resolve(process.argv[2] || "target/release/codex-nav");
const tabs = new Set();
const services = new Set();
const roots = [];
const pause = (ms) =>
  new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
const hash = (data) => createHash("sha256").update(data).digest("hex");
async function cdp(endpoint, body) {
  const response = await fetch(`http://localhost:3456${endpoint}`, {
    ...(body === undefined ? {} : { method: "POST", body }),
    signal: AbortSignal.timeout(55000),
  });
  const result = await response.json();
  if (!response.ok || result.error)
    throw new Error(`浏览器代理失败 ${endpoint.split("?")[0]}`);
  return result;
}
async function evaluate(target, code) {
  const result = await cdp(
    `/eval?target=${target}`,
    `(async()=>{try{return await (${code})}catch(error){return {qaError:error.message}}})()`,
  );
  if (result.value?.qaError || !Object.hasOwn(result, "value"))
    throw new Error(JSON.stringify(result));
  return result.value;
}
async function wait(target, condition, label) {
  const passed = await evaluate(
    target,
    `(async()=>{const until=Date.now()+15000;while(Date.now()<until){if(${condition})return true;await new Promise(r=>setTimeout(r,100))}return false})()`,
  );
  assert.ok(
    passed,
    `${label}：${passed ? "" : await evaluate(target, "document.body.innerText.slice(-1400)")}`,
  );
}
const click = (target, selector) => cdp(`/click?target=${target}`, selector);
async function stop(service) {
  if (service.exitCode !== null || service.signalCode !== null) return;
  service.kill("SIGINT");
  await Promise.race([once(service, "exit"), pause(5000)]);
  if (service.exitCode === null && service.signalCode === null) {
    service.kill("SIGTERM");
    await Promise.race([once(service, "exit"), pause(5000)]);
  }
}
async function start(home, env) {
  const service = spawn(
    binary,
    ["--web", "--port", "0", "--no-open", "--codex-home", home],
    { env, stdio: ["ignore", "pipe", "pipe"] },
  );
  services.add(service);
  let stderr = "";
  service.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  const url = await new Promise((resolve, reject) => {
    let output = "";
    const timeout = setTimeout(
      () => reject(new Error(`服务启动超时：${stderr}`)),
      12000,
    );
    service.once("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    service.once("exit", () => {
      clearTimeout(timeout);
      reject(new Error("验收服务提前退出"));
    });
    service.stdout.on("data", (chunk) => {
      output += chunk;
      const match = output.match(
        /http:\/\/127\.0\.0\.1:\d+\/#token=[a-zA-Z0-9_-]+/,
      );
      if (match) {
        clearTimeout(timeout);
        resolve(match[0]);
      }
    });
  });
  return { service, url };
}
async function resize(target, width) {
  const title = `FavoritesQA-${process.pid}-${width}-${Date.now()}`;
  const oldTitle = await evaluate(
    target,
    `(()=>{const previous=document.title;document.title=${JSON.stringify(title)};window.resizeTo(${width}+outerWidth-innerWidth,850+outerHeight-innerHeight);return previous})()`,
  );
  try {
    await pause(350);
    const { stdout } = await execute(
      "xdotool",
      ["search", "--onlyvisible", "--name", `^${title}( - Google Chrome)?$`],
      { timeout: 3000 },
    );
    const ids = stdout.trim().split(/\s+/).filter(Boolean);
    if (ids.length === 1 && /^\d+$/.test(ids[0])) {
      const { stdout: actualTitle } = await execute(
        "xdotool",
        ["getwindowname", ids[0]],
        { timeout: 3000 },
      );
      assert.ok(
        [title, `${title} - Google Chrome`].includes(actualTitle.trim()),
        "仅调整自己的窗口",
      );
      const border = await evaluate(
        target,
        "({width:outerWidth-innerWidth,height:outerHeight-innerHeight})",
      );
      await execute(
        "xdotool",
        [
          "windowsize",
          "--sync",
          ids[0],
          String(width + border.width),
          String(850 + border.height),
        ],
        { timeout: 3000 },
      );
    }
  } catch {
    // 窗口管理器可能限制几何，下面核对实际视口。
  } finally {
    await evaluate(
      target,
      `(()=>{document.title=${JSON.stringify(oldTitle)};return true})()`,
    );
  }
  await pause(400);
  const actual = await evaluate(target, "innerWidth");
  assert.ok(width === 390 ? actual === 390 : actual >= 1280, "实际响应式宽度");
  console.log(`功能验收：请求 ${width}px，实际 ${actual}px`);
  return actual;
}
async function launch(launcher, url, width) {
  const qaUrl = url.replace("/#", `/?favoritesqa=${width}-${Date.now()}#`);
  await evaluate(
    launcher,
    `(()=>{document.body.replaceChildren();const b=document.createElement('button');b.id='launch';b.textContent='打开功能验收';b.onclick=()=>window.open(${JSON.stringify(qaUrl)},'favorites-${width}-${Date.now()}','popup,width=${width},height=850');document.body.append(b);return true})()`,
  );
  await cdp(`/clickAt?target=${launcher}`, "#launch");
  for (let i = 0; i < 40; i++) {
    const target = (await cdp("/targets")).find((tab) =>
      tab.url.startsWith(qaUrl.split("#")[0]),
    )?.targetId;
    if (target) {
      tabs.add(target);
      await resize(target, width);
      return target;
    }
    await pause(100);
  }
  throw new Error("未找到自建验收窗口");
}

const record = (type, payload, timestamp = "2020-01-01T00:00:00Z") =>
  JSON.stringify({ type, payload, timestamp }) + "\n";
const fixture = (id, title, cwd, count) =>
  record("session_meta", {
    id,
    cwd,
    source: "cli",
    originator: "codex_cli_rs",
  }) +
  Array.from(
    { length: count },
    (_, index) =>
      record("turn_context", { turn_id: `${id}-turn-${index}` }) +
      record("event_msg", {
        type: "user_message",
        message:
          index === 0 ? title : `收藏定位问题 ${index + 1}：历史问题的稳定身份`,
        images: [],
        local_images: [],
      }),
  ).join("");
async function api(url, endpoint, body) {
  const parsed = new URL(url);
  const response = await fetch(new URL(endpoint, parsed), {
    method: body ? "POST" : "GET",
    headers: {
      "X-Codex-Nav-Token": new URLSearchParams(parsed.hash.slice(1)).get(
        "token",
      ),
      ...(body
        ? { "Content-Type": "application/json", Origin: parsed.origin }
        : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  assert.ok(response.ok, `${endpoint}: ${response.status}`);
  return response.json();
}
async function ready(url, endpoint) {
  for (let i = 0; i < 100; i++) {
    const data = await api(url, endpoint);
    if (!data.loading) return data;
    await pause(100);
  }
  throw new Error(`读取超时 ${endpoint}`);
}
async function sidebar(target) {
  await evaluate(
    target,
    `(()=>{if(innerWidth<1280&&!document.querySelector('.sidebar-open'))document.querySelector('[aria-label="打开会话列表"]').click();return true})()`,
  );
}
async function button(target, scope, text) {
  await evaluate(
    target,
    `(()=>{const button=[...document.querySelectorAll(${JSON.stringify(scope + " button")})].find(item=>item.textContent.trim().startsWith(${JSON.stringify(text)}));if(!button)throw new Error('未找到按钮');button.click();return true})()`,
  );
}
async function query(target, value) {
  await evaluate(
    target,
    `(()=>{const input=document.querySelector('.session-filter input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,${JSON.stringify(value)});input.dispatchEvent(new Event('input',{bubbles:true}));return true})()`,
  );
}
try {
  const launcher = (await cdp("/new?url=about%3Ablank")).targetId;
  tabs.add(launcher);
  for (const width of [1848, 390]) {
    const root = await fs.mkdtemp(
      path.join(os.tmpdir(), `codex-favorites-${width}-`),
    );
    roots.push(root);
    const home = path.join(root, "codex");
    const directory = path.join(home, "sessions", "2020", "01", "01");
    await fs.mkdir(directory, { recursive: true });
    const oldId = randomUUID(),
      currentId = randomUUID();
    const oldSource = fixture(
      oldId,
      "早期研究合成会话",
      "/synthetic/OldProject",
      3,
    );
    const newSource = fixture(
      currentId,
      "当前合成会话",
      "/synthetic/current",
      2,
    );
    const oldPath = path.join(
      directory,
      `rollout-2020-01-01T00-00-00-${oldId}.jsonl`,
    );
    const today = new Date().toISOString().slice(0, 10).split("-");
    const currentDirectory = path.join(home, "sessions", ...today);
    await fs.mkdir(currentDirectory, { recursive: true });
    const newPath = path.join(
      currentDirectory,
      `rollout-${today.join("-")}T00-01-00-${currentId}.jsonl`,
    );
    await fs.writeFile(oldPath, oldSource);
    await fs.writeFile(newPath, newSource);
    await fs.utimes(oldPath, new Date("2020-01-01"), new Date("2020-01-01"));
    const running = await start(home, {
      ...process.env,
      CODEX_HOME: home,
      XDG_CONFIG_HOME: path.join(root, "config"),
      XDG_DATA_HOME: path.join(root, "data"),
    });
    const listing = await ready(running.url, "/api/sessions?all=1");
    const old = listing.sessions.find((item) => item.id === oldId);
    const current = listing.sessions.find((item) => item.id === currentId);
    assert.ok(old && current, "合成会话可读");
    const graph = await ready(running.url, `/api/trail/session/${old.key}`);
    await api(running.url, `/api/session/${old.key}/favorite`, {
      favorite: true,
      node_id: "q2",
      generation: graph.generation,
    });
    const catalog = await ready(running.url, "/api/trail/favorites");
    assert.equal(catalog.results.length, 1);
    assert.equal(catalog.results[0].cwd, "/synthetic/OldProject");
    assert.equal(
      (await ready(running.url, "/api/sessions?all=1")).sessions.find(
        (item) => item.id === oldId,
      ).favorite,
      false,
      "父会话未收藏",
    );
    const recent = await ready(running.url, "/api/sessions?all=0");
    assert.ok(
      !recent.sessions.some((item) => item.id === oldId),
      "收藏问题来自普通近期列表以外",
    );
    const target = await launch(launcher, running.url, width);
    await wait(
      target,
      "document.querySelectorAll('.session-item').length===2&&document.querySelectorAll('.qt-card').length>0",
      "画布加载",
    );
    await sidebar(target);
    await button(target, ".session-filters", "收藏");
    await wait(
      target,
      "document.querySelectorAll('.favorite-question').length===1",
      "未收藏父会话的问题独立显示",
    );
    assert.equal(
      await evaluate(
        target,
        "document.querySelectorAll('.session-item').length",
      ),
      0,
      "收藏会话为空",
    );
    const text = await evaluate(
      target,
      "document.querySelector('.favorite-question').innerText",
    );
    for (const phrase of [
      "早期研究合成会话",
      "/synthetic/OldProject",
      "问题 2",
    ])
      assert.ok(text.includes(phrase));
    await query(target, "/SYNTHETIC/oldproject");
    await wait(
      target,
      "document.querySelectorAll('.favorite-question').length===1",
      "路径搜索不区分大小写",
    );
    await query(target, "完全不匹配");
    await wait(
      target,
      "!document.querySelector('.favorite-question')&&document.querySelector('.favorite-empty')?.innerText.includes('没有匹配')",
      "无结果空态",
    );
    await query(target, "历史问题");
    await wait(
      target,
      "document.querySelectorAll('.favorite-question').length===1",
      "问题正文搜索",
    );
    await query(target, "");
    await button(target, ".favorite-categories", "会话");
    await wait(
      target,
      "!document.querySelector('.favorite-question')&&document.querySelector('.favorite-empty')?.innerText.includes('还没有收藏会话')",
      "类型切换",
    );
    await button(target, ".favorite-categories", "问题");
    await wait(
      target,
      "!!document.querySelector('.favorite-question')",
      "问题类型",
    );
    await cdp(
      `/screenshot?target=${target}&file=${encodeURIComponent(path.join(root, "favorite-library.png"))}`,
    );
    // 模拟旧会话读取较慢；切换到另一个会话后不能再跳回收藏问题。
    await evaluate(
      target,
      `(()=>{window.__favoriteFetch=window.fetch;window.fetch=async(input,init)=>{if(String(input).includes(${JSON.stringify("/api/trail/session/" + old.key)}))await new Promise(resolve=>setTimeout(resolve,800));return window.__favoriteFetch(input,init)};return true})()`,
    );
    await click(target, ".favorite-question");
    await sidebar(target);
    await button(target, ".session-filters", "全部");
    await evaluate(
      target,
      `(()=>{[...document.querySelectorAll('.session-item')].find(item=>item.textContent.includes('当前合成会话')).click();return true})()`,
    );
    await wait(
      target,
      "document.querySelectorAll('.qt-card').length===2&&document.querySelector('.session-heading-title')?.innerText.includes('当前合成会话')",
      "后选会话保持优先",
    );
    await pause(1200);
    assert.equal(
      await evaluate(target, "document.querySelectorAll('.qt-card').length"),
      2,
      "迟到收藏响应不覆盖后选会话",
    );
    await evaluate(
      target,
      "(()=>{window.fetch=window.__favoriteFetch;return true})()",
    );
    await sidebar(target);
    await button(target, ".session-filters", "收藏");
    await wait(
      target,
      "!!document.querySelector('.favorite-question')",
      "再次进入收藏",
    );
    await button(target, ".favorite-categories", "问题");
    await click(target, ".favorite-question");
    await wait(
      target,
      "document.querySelector('.qt-card.is-selected')?.getAttribute('data-question-id')==='q2'&&document.querySelectorAll('.qt-card').length===3",
      "跨会话精确定位",
    );
    if (width === 390)
      assert.equal(
        await evaluate(target, "!!document.querySelector('.sidebar-open')"),
        false,
        "手机定位自动收起侧栏",
      );
    await wait(
      target,
      "!!document.querySelector('.detail-panel.has-selection')",
      "问题详情打开",
    );
    await click(target, '[data-question-id="q2"] .qt-card-favorite');
    await wait(
      target,
      "!document.querySelector('[data-question-id=\"q2\"].is-favorite')",
      "取消问题收藏",
    );
    await sidebar(target);
    await wait(
      target,
      "!document.querySelector('.favorite-question')&&document.querySelector('.favorite-empty')?.innerText.includes('还没有收藏问题')",
      "收藏入口即时移除",
    );
    // 保留旧收藏行，模拟用户点击前源文件已替换、q2 被另一个问题复用。
    const beforeReplacement = await ready(
      running.url,
      `/api/trail/session/${old.key}`,
    );
    await api(running.url, `/api/session/${old.key}/favorite`, {
      favorite: true,
      node_id: "q2",
      generation: beforeReplacement.generation,
    });
    const savedCatalog = await ready(running.url, "/api/trail/favorites");
    await evaluate(
      target,
      `(()=>{window.__favoriteFetch=window.fetch;window.fetch=(input,init)=>String(input).includes('/api/trail/favorites')?Promise.resolve(new Response(${JSON.stringify(JSON.stringify(savedCatalog))},{status:200,headers:{'Content-Type':'application/json'}})):window.__favoriteFetch(input,init);document.querySelector('[aria-label="重新扫描会话"]').click();return true})()`,
    );
    await wait(
      target,
      "!!document.querySelector('.favorite-question')",
      "模拟缓存中的旧收藏",
    );
    await fs.writeFile(
      `${oldPath}.replacement`,
      oldSource.replace(`${oldId}-turn-1`, `${oldId}-replacement-turn`),
    );
    await fs.rename(oldPath, `${oldPath}.original`);
    await fs.rename(`${oldPath}.replacement`, oldPath);
    await api(running.url, `/api/trail/session/${old.key}?refresh=1`);
    await ready(running.url, `/api/trail/session/${old.key}`);
    await click(target, ".favorite-question");
    await wait(
      target,
      "document.body.innerText.includes('收藏的问题暂时无法定位')",
      "旧收藏缺失提示",
    );
    assert.equal(
      await evaluate(
        target,
        "!!document.querySelector('.qt-card.is-selected')",
      ),
      false,
      "临时q2复用不会错挂收藏",
    );
    await evaluate(
      target,
      "(()=>{window.fetch=window.__favoriteFetch;return true})()",
    );
    await fs.rename(oldPath, `${oldPath}.replaced-result`);
    await fs.rename(`${oldPath}.original`, oldPath);
    assert.equal(
      hash(await fs.readFile(oldPath)),
      hash(oldSource),
      "收藏不修改旧会话",
    );
    assert.equal(
      hash(await fs.readFile(newPath)),
      hash(newSource),
      "收藏不修改当前会话",
    );
    assert.equal(
      await evaluate(
        target,
        "document.documentElement.scrollWidth<=innerWidth",
      ),
      true,
      "页面没有横向溢出",
    );
    await stop(running.service);
    await cdp(`/close?target=${target}`);
    tabs.delete(target);
    console.log(`收藏入口 ${width}px 验收通过：${root}`);
  }
} finally {
  for (const service of services) await stop(service);
  for (const target of tabs)
    await cdp(`/close?target=${target}`).catch(() => {});
  console.log(`合成材料保留：${roots.join("、")}`);
}
