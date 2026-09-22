// 仅在隔离合成 CODEX_HOME 与自建 Chrome 窗口中验收可拖动面板宽度。
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
const windows = new Map();
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
    `(async()=>{const until=Date.now()+45000;while(Date.now()<until){if(${condition})return true;await new Promise(r=>setTimeout(r,100))}return false})()`,
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
  const title = `PanelsQA-${process.pid}-${width}-${Date.now()}`;
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
      windows.set(target, ids[0]);
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
  } catch (error) {
    console.log("窗口查找失败", error.message);
    // 窗口管理器可能限制几何，下面核对实际视口。
  } finally {
    await evaluate(
      target,
      `(()=>{document.title=${JSON.stringify(oldTitle)};return true})()`,
    );
  }
  await pause(400);
  const actual = await evaluate(target, "innerWidth");
  assert.ok(actual === width, "实际响应式宽度");
  console.log(`功能验收：请求 ${width}px，实际 ${actual}px`);
  return actual;
}
async function launch(launcher, url, width) {
  const qaUrl = url.replace("/#", `/?panelsqa=${width}-${Date.now()}#`);
  await evaluate(
    launcher,
    `(()=>{document.body.replaceChildren();const b=document.createElement('button');b.id='launch';b.textContent='打开功能验收';b.onclick=()=>window.open(${JSON.stringify(qaUrl)},'panels-${width}-${Date.now()}','popup,width=${width},height=850');document.body.append(b);return true})()`,
  );
  await cdp(`/clickAt?target=${launcher}`, "#launch");
  for (let i = 0; i < 40; i++) {
    const target = (await cdp("/targets")).find((tab) =>
      tab.url.startsWith(qaUrl.split("#")[0]),
    )?.targetId;
    if (target) {
      tabs.add(target);
      await wait(
        target,
        "document.querySelectorAll('.qt-card').length>0",
        "会话载入后核对窗口身份",
      );
      await pause(300);
      await resize(target, width);
      return target;
    }
    await pause(100);
  }
  throw new Error("未找到自建验收窗口");
}

// 原生指针仅发往自己的窗口；不重新连接浏览器，避免触发新的调试授权。
async function protocol(target, method, params = {}) {
  assert.ok(tabs.has(target), "仅操作自己创建的 target");
  if (method === "Page.reload")
    return evaluate(target, "(()=>{location.reload();return true})()");
  if (method === "Input.dispatchKeyEvent")
    return evaluate(
      target,
      `(()=>{document.activeElement.dispatchEvent(new KeyboardEvent(${JSON.stringify(params.type === "keyDown" ? "keydown" : "keyup")},{key:${JSON.stringify(params.key)},shiftKey:${params.modifiers === 8},bubbles:true,cancelable:true}));return true})()`,
    );
  assert.equal(method, "Input.dispatchMouseEvent");
  assert.ok(windows.has(target), "已核对自建窗口身份");
  if (params.type === "mousePressed") {
    await execute(
      "xdotool",
      ["windowactivate", "--sync", windows.get(target)],
      { timeout: 3000 },
    );
    await pause(120);
  }
  const position = await evaluate(
    target,
    "({x:screenX+(outerWidth-innerWidth)/2,y:screenY+outerHeight-innerHeight})",
  );
  await execute("xdotool", [
    "mousemove",
    String(Math.round(position.x + params.x)),
    String(Math.round(position.y + params.y)),
  ]);
  if (params.type === "mousePressed")
    await execute("xdotool", ["mousedown", "1"]);
  if (params.type === "mouseReleased")
    await execute("xdotool", ["mouseup", "1"]);
}
const panelSelector = (side) => `.panel-resize-${side}`;
const number = (target, side) =>
  evaluate(
    target,
    `Number(document.querySelector('${panelSelector(side)}')?.getAttribute('aria-valuenow'))`,
  );
const layout = (target) =>
  evaluate(
    target,
    `(()=>{const width=s=>document.querySelector(s)?.getBoundingClientRect().width;return {sidebar:width('.session-sidebar'),detail:width('.detail-panel'),canvas:width('.react-flow'),selected:document.querySelector('.qt-card.is-selected')?.dataset.questionId,transform:document.querySelector('.react-flow__viewport')?.style.transform,stored:localStorage.getItem('questionTrail.panelWidths'),overflow:document.documentElement.scrollWidth>innerWidth}})()`,
  );
async function key(target, side, value, shift = false) {
  await evaluate(
    target,
    `(()=>{document.querySelector('${panelSelector(side)}').focus();return true})()`,
  );
  await protocol(target, "Input.dispatchKeyEvent", {
    type: "keyDown",
    key: value,
    code: value,
    modifiers: shift ? 8 : 0,
  });
  await protocol(target, "Input.dispatchKeyEvent", {
    type: "keyUp",
    key: value,
    code: value,
  });
  await pause(80);
}
async function drag(target, side, dx, outcome = "release") {
  if (outcome !== "immediate")
    await wait(
      target,
      "(await (async()=>{let previous='',stable=0;for(let i=0;i<35;i++){await new Promise(r=>setTimeout(r,100));const now=document.querySelector('.react-flow__viewport')?.style.transform;stable=now===previous?stable+1:0;previous=now;if(stable>=3)return true}return false})())",
      "前序定位动画完成",
    );
  const center = await evaluate(
    target,
    `(()=>{const r=document.querySelector('${panelSelector(side)}').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()`,
  );
  await protocol(target, "Input.dispatchMouseEvent", {
    type: "mouseMoved",
    ...center,
  });
  await protocol(target, "Input.dispatchMouseEvent", {
    type: "mousePressed",
    button: "left",
    buttons: 1,
    clickCount: 1,
    ...center,
  });
  await pause(60);
  assert.ok(
    await evaluate(
      target,
      "!!document.querySelector('.panel-resize-handle.is-dragging')",
    ),
    "真实指针按下已进入拖动",
  );
  const previous = await layout(target);
  for (let i = 1; i <= 6; i++) {
    await protocol(target, "Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: center.x + (dx * i) / 6,
      y: center.y,
      button: "left",
      buttons: 1,
    });
    await pause(35);
  }
  const during = await layout(target);
  assert.equal(
    during.transform,
    previous.transform,
    "指针拖动期间画布视角不跳变",
  );
  assert.equal(during.selected, previous.selected, "指针拖动期间保留选中问题");
  if (outcome === "breakpoint") await resize(target, 900);
  if (outcome === "escape") await key(target, side, "Escape");
  if (outcome === "blur")
    await evaluate(
      target,
      "(()=>{document.querySelector('input').focus();return true})()",
    );
  if (outcome === "cancel")
    await evaluate(
      target,
      `(()=>{const el=document.querySelector('${panelSelector(side)}');el.dispatchEvent(new PointerEvent('pointercancel',{bubbles:true,pointerId:1}));return true})()`,
    );
  await protocol(target, "Input.dispatchMouseEvent", {
    type: "mouseReleased",
    x: center.x + dx,
    y: center.y,
    button: "left",
    buttons: 0,
    clickCount: 1,
  });
  await pause(500);
  return { previous, during, after: await layout(target) };
}
async function doubleClick(target, side) {
  const center = await evaluate(
    target,
    `(()=>{const r=document.querySelector('${panelSelector(side)}').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()`,
  );
  await protocol(target, "Input.dispatchMouseEvent", {
    type: "mouseMoved",
    ...center,
  });
  await execute("xdotool", ["click", "--repeat", "2", "--delay", "100", "1"]);
  await pause(150);
}
const picture = (target, root, name) =>
  cdp(
    `/screenshot?target=${target}&file=${encodeURIComponent(path.join(root, name))}`,
  );
const record = (type, payload, seconds = 0) =>
  JSON.stringify({
    timestamp: new Date(
      Date.UTC(2026, 8, 23, 9) + seconds * 1000,
    ).toISOString(),
    type,
    payload,
  }) + "\n";

try {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "codex-panels-"));
  roots.push(root);
  const home = path.join(root, "codex");
  const directory = path.join(home, "sessions", "2026", "09", "23");
  await fs.mkdir(directory, { recursive: true });
  const id = randomUUID();
  let source = record("session_meta", {
    id,
    cwd: "/synthetic/panel-layout",
    source: "cli",
    originator: "codex_cli_rs",
  });
  for (let n = 1; n <= 16; n++) {
    source += record(
      "event_msg",
      {
        type: "user_message",
        message:
          n === 1
            ? "灵活布局验收会话"
            : `问题 ${n}：保留阅读位置并调整左右栏宽`,
        images: [],
        local_images: [],
      },
      n * 10,
    );
    if (n === 4) source += record("compacted", { trigger: "auto" }, n * 10 + 1);
  }
  const sourcePath = path.join(
    directory,
    `rollout-2026-09-23T09-00-00-${id}.jsonl`,
  );
  await fs.writeFile(sourcePath, source);
  const running = await start(home, {
    ...process.env,
    CODEX_HOME: home,
    XDG_CONFIG_HOME: path.join(root, "config"),
    XDG_DATA_HOME: path.join(root, "data"),
  });
  const launcher = (await cdp("/new?url=about%3Ablank")).targetId;
  tabs.add(launcher);
  const target = await launch(launcher, running.url, 1848);
  await wait(
    target,
    "document.querySelectorAll('.qt-card').length===16&&document.querySelectorAll('[role=separator]').length===2",
    "会话和拖动边界已加载",
  );
  await click(target, '[data-question-id="q2"]');
  await pause(600);
  assert.equal(await number(target, "sidebar"), 250);
  assert.equal(await number(target, "detail"), 360);
  await evaluate(
    target,
    "(()=>{[...document.querySelectorAll('.layout-controls button')].find(b=>b.textContent.trim()==='横向').click();return true})()",
  );
  await pause(600);
  await drag(target, "sidebar", 90);
  assert.equal(await number(target, "sidebar"), 340, "真实鼠标向右扩大左栏");
  await drag(target, "detail", -140);
  assert.equal(await number(target, "detail"), 500, "真实鼠标向左扩大右栏");
  assert.equal((await layout(target)).selected, "q2");
  const stored = (await layout(target)).stored;
  await protocol(target, "Page.reload");
  await wait(
    target,
    "document.querySelectorAll('.qt-card').length===16",
    "刷新重新载入会话",
  );
  assert.equal(await number(target, "sidebar"), 340, "刷新保留左栏宽度");
  assert.equal(await number(target, "detail"), 500, "刷新保留右栏宽度");
  await click(target, '[data-question-id="q2"]');
  await pause(600);
  for (const outcome of ["escape", "blur", "cancel"]) {
    await drag(target, "sidebar", 40, outcome);
    assert.equal(
      await number(target, "sidebar"),
      340,
      `${outcome} 恢复拖动前宽度`,
    );
    assert.equal(
      (await layout(target)).stored,
      stored,
      `${outcome} 不保存中间宽度`,
    );
  }
  await drag(target, "sidebar", 35, "breakpoint");
  assert.equal((await layout(target)).stored, stored, "拖动时跨断点回滚首选");
  await resize(target, 1848);
  assert.equal(await number(target, "sidebar"), 340);
  await key(target, "sidebar", "ArrowRight");
  assert.equal(await number(target, "sidebar"), 350);
  await key(target, "detail", "ArrowLeft", true);
  assert.equal(await number(target, "detail"), 550);
  await key(target, "sidebar", "Home");
  assert.equal(await number(target, "sidebar"), 220);
  assert.ok(
    await evaluate(
      target,
      "(()=>{const e=document.querySelector('.brand');return !e||e.scrollWidth<=e.clientWidth})()",
    ),
    "220px品牌不溢出",
  );
  await key(target, "sidebar", "End");
  assert.equal(await number(target, "sidebar"), 420);
  await key(target, "detail", "End");
  assert.equal(await number(target, "detail"), 640);
  await resize(target, 1280);
  assert.ok((await layout(target)).canvas >= 479, "1280px 保留至少480px画布");
  assert.equal((await layout(target)).overflow, false);
  const preferredAtSmall = (await layout(target)).stored;
  await drag(target, "sidebar", 0);
  assert.equal(
    (await layout(target)).stored,
    preferredAtSmall,
    "压缩布局零位移不覆写宽屏首选",
  );
  await resize(target, 1848);
  assert.equal(await number(target, "sidebar"), 420, "恢复宽窗口保留首选左栏");
  assert.equal(await number(target, "detail"), 640, "恢复宽窗口保留首选右栏");
  await doubleClick(target, "sidebar");
  assert.equal(await number(target, "sidebar"), 250, "双击恢复默认");
  await key(target, "detail", "Enter");
  assert.equal(await number(target, "detail"), 360, "Enter恢复默认");
  // 立即在上一轮宽度变化/聚焦动画之后接续拖动。
  await click(target, '[data-question-id="q12"]');
  await drag(target, "detail", -120, "immediate");
  await evaluate(
    target,
    "(()=>{[...document.querySelectorAll('button')].find(b=>b.textContent.trim().startsWith('会话事件 ')).click();return true})()",
  );
  await wait(
    target,
    "!!document.querySelector('.session-event-row')",
    "会话事件列表可用",
  );
  await click(target, ".session-event-row");
  await wait(
    target,
    "!!document.querySelector('.event-detail')",
    "事件详情可用",
  );
  assert.equal(await number(target, "detail"), 480, "事件详情共用用户设置宽度");
  await evaluate(
    target,
    "(()=>{const e=document.querySelector('select[aria-label=\"主题\"]');e.value='dark';e.dispatchEvent(new Event('change',{bubbles:true}));return true})()",
  );
  await picture(target, root, "desktop-dark.png");
  await evaluate(
    target,
    "(()=>{document.querySelector('select[aria-label=\"主题\"]').value='light';document.querySelector('select[aria-label=\"主题\"]').dispatchEvent(new Event('change',{bubbles:true}));return true})()",
  );
  await pause(200);
  await picture(target, root, "desktop-light.png");
  for (const width of [900, 390]) {
    await resize(target, width);
    assert.equal(
      await evaluate(
        target,
        "[...document.querySelectorAll('[role=separator]')].filter(e=>e.getBoundingClientRect().width>0).length",
      ),
      0,
      "窄屏不展示拖动分隔线",
    );
    assert.equal(
      (await layout(target)).overflow,
      false,
      "窄屏没有页面横向溢出",
    );
    await picture(target, root, `responsive-${width}.png`);
  }
  await resize(target, 1848);
  const remembered = (await layout(target)).stored;
  await click(target, ".collapse-control");
  assert.equal(
    await evaluate(target, "!!document.querySelector('.panel-resize-sidebar')"),
    false,
    "折叠隐藏左分隔线",
  );
  await click(target, '[aria-label="打开会话列表"]');
  assert.equal((await layout(target)).stored, remembered, "展开保留首选宽度");
  await click(target, '[aria-label="画布全屏"]');
  assert.equal(
    await evaluate(
      target,
      "document.querySelectorAll('[role=separator]').length",
    ),
    0,
    "全屏隐藏分隔线",
  );
  await click(target, '[aria-label="退出画布全屏"]');
  assert.equal((await layout(target)).stored, remembered);
  await evaluate(
    target,
    "(()=>{localStorage.setItem('questionTrail.panelWidths','{broken');return true})()",
  );
  await protocol(target, "Page.reload");
  await wait(
    target,
    "document.querySelectorAll('[role=separator]').length===2",
    "损坏偏好仍正常打开",
  );
  assert.equal(await number(target, "sidebar"), 250);
  assert.equal(await number(target, "detail"), 360);
  assert.equal(
    hash(await fs.readFile(sourcePath)),
    hash(source),
    "所有验收不修改会话原文件",
  );
  console.log(
    `面板验收通过：拖动、取消、键盘、持久化、断点、事件详情、坏值降级、截图；材料 ${root}`,
  );
} finally {
  for (const service of services) await stop(service);
  for (const target of tabs)
    await cdp(`/close?target=${target}`).catch(() => {});
  console.log(`合成验收材料保留：${roots.join("、")}`);
}
