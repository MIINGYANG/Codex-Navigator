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
    `(async()=>{const until=Date.now()+45000;while(Date.now()<until){if(${condition})return true;await new Promise(r=>setTimeout(r,100))}return false})()`,
  );
  assert.ok(
    passed,
    `${label}：${passed ? "" : await evaluate(target, "document.body.innerText.slice(-1400)")}`,
  );
}
const click = (target, selector) => cdp(`/click?target=${target}`, selector);
const view = (target) =>
  evaluate(
    target,
    "({selected:document.querySelector('.qt-card.is-selected')?.getAttribute('data-question-id'),transform:document.querySelector('.react-flow__viewport')?.style.transform})",
  );
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
  const title = `FeaturesQA-${process.pid}-${width}-${Date.now()}`;
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
  const qaUrl = url.replace("/#", `/?featuresqa=${width}-${Date.now()}#`);
  await evaluate(
    launcher,
    `(()=>{document.body.replaceChildren();const b=document.createElement('button');b.id='launch';b.textContent='打开功能验收';b.onclick=()=>window.open(${JSON.stringify(qaUrl)},'features-${width}-${Date.now()}','popup,width=${width},height=850');document.body.append(b);return true})()`,
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

const record = (type, payload, seconds = 0) =>
  JSON.stringify({
    timestamp: new Date(Date.UTC(2026, 8, 17, 9, 0, seconds)).toISOString(),
    type,
    payload,
  }) + "\n";
const question = (number) =>
  record(
    "event_msg",
    {
      type: "user_message",
      message:
        number === 1
          ? "画布整理合成会话"
          : `问题 ${number}：验证可靠事件与清晰导航`,
      images: [],
      local_images: [],
    },
    number * 10,
  );
function command(id, cmd, output, seconds) {
  return (
    record(
      "response_item",
      {
        type: "function_call",
        name: "exec_command",
        call_id: id,
        arguments: JSON.stringify({
          cmd,
          workdir: "/synthetic/canvas-repository",
        }),
      },
      seconds,
    ) +
    record(
      "response_item",
      {
        type: "function_call_output",
        call_id: id,
        output: { exit_code: 0, output },
      },
      seconds + 1,
    )
  );
}
async function buttonText(target, scope, text) {
  return evaluate(
    target,
    `(()=>{const b=[...document.querySelectorAll(${JSON.stringify(`${scope} button`)})].find(e=>e.textContent.trim()===${JSON.stringify(text)});if(!b)throw new Error('按钮不存在');b.click();return true})()`,
  );
}
const picture = (target, root, filename) =>
  cdp(
    `/screenshot?target=${target}&file=${encodeURIComponent(path.join(root, filename))}`,
  );
async function api(url, pathname, body, authenticate = true) {
  const parsed = new URL(url);
  const token = new URLSearchParams(parsed.hash.slice(1)).get("token");
  const response = await fetch(new URL(pathname, parsed), {
    method: body ? "POST" : "GET",
    headers: {
      ...(authenticate ? { "X-Codex-Nav-Token": token } : {}),
      ...(body
        ? { "Content-Type": "application/json", Origin: parsed.origin }
        : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  return { status: response.status, body: await response.json() };
}
async function sidebar(target) {
  if (
    await evaluate(
      target,
      "innerWidth<1280&&!document.querySelector('.app-shell.sidebar-open')",
    )
  ) {
    const available = await evaluate(
      target,
      "!!document.querySelector('[aria-label=\"打开会话列表\"]')",
    );
    if (available) await click(target, '[aria-label="打开会话列表"]');
  }
}
async function closeSidebar(target) {
  const selector = '[aria-label="关闭会话列表"]';
  if (
    await evaluate(
      target,
      `!!document.querySelector(${JSON.stringify(selector)})`,
    )
  )
    await click(target, selector);
}
async function openEvents(target) {
  await evaluate(
    target,
    "(()=>{const e=[...document.querySelectorAll('button')].find(e=>e.textContent.trim().startsWith('会话事件 '));if(!e)throw new Error('缺少会话事件入口');e.click();return true})()",
  );
  await wait(
    target,
    "!!document.querySelector('dialog[aria-label=\"会话事件\"][open]')",
    "事件列表打开",
  );
}
async function eventDetail(target, title) {
  await openEvents(target);
  await evaluate(
    target,
    `(()=>{const e=[...document.querySelectorAll('.session-event-row')].find(e=>e.querySelector('strong')?.textContent===${JSON.stringify(title)});if(!e)throw new Error('事件不存在');e.click();return true})()`,
  );
  await wait(
    target,
    "!!document.querySelector('.event-detail')",
    "事件详情可见",
  );
}
async function closeDetail(target) {
  if (
    await evaluate(
      target,
      "!!document.querySelector('[aria-label=\"关闭事件详情\"]')",
    )
  )
    await click(target, '[aria-label="关闭事件详情"]');
  else if (
    await evaluate(
      target,
      "!!document.querySelector('[aria-label=\"关闭问题详情\"]')",
    )
  )
    await click(target, '[aria-label="关闭问题详情"]');
}
const coordinates = (target) =>
  evaluate(
    target,
    "Object.fromEntries([...document.querySelectorAll('.react-flow__node-question')].map(e=>[e.dataset.id,e.style.transform]))",
  );

try {
  const launcher = (await cdp("/new?url=about%3Ablank")).targetId;
  tabs.add(launcher);
  for (const width of process.env.FEATURE_QA_WIDTH
    ? [Number(process.env.FEATURE_QA_WIDTH)]
    : [1848, 390]) {
    const root = await fs.mkdtemp(
      path.join(os.tmpdir(), `codex-features-${width}-`),
    );
    roots.push(root);
    const home = path.join(root, "codex");
    const env = {
      ...process.env,
      CODEX_HOME: home,
      XDG_CONFIG_HOME: path.join(root, "config"),
      XDG_DATA_HOME: path.join(root, "data"),
    };
    const directory = path.join(home, "sessions", "2026", "09", "17");
    await fs.mkdir(directory, { recursive: true });
    const id = randomUUID();
    let source = record("session_meta", {
      id,
      cwd: "/synthetic/canvas-repository",
      source: "cli",
      originator: "codex_cli_rs",
    });
    for (let number = 1; number <= 12; number++) {
      source += question(number);
      if (number === 2)
        source +=
          command(
            "commit-version",
            "git commit -m demo",
            "[main abc1234] demo\n 1 file changed",
            22,
          ) + command("tag", "git tag v3.1.0 abc1234", "", 25);
      if (number === 3) source += record("compacted", { trigger: "auto" }, 33);
      if (number === 5)
        source += command(
          "commit-no-version",
          "git commit -m followup",
          "[feature/canvas def5678] followup\n 2 files changed",
          52,
        );
      if (number === 6)
        source += record("compacted", { trigger: "manual" }, 63);
      if (number === 12) source += record("compacted", {}, 123);
    }
    const sourcePath = path.join(
      directory,
      `rollout-2026-09-17T09-00-00-${id}.jsonl`,
    );
    await fs.writeFile(sourcePath, source);
    const otherId = randomUUID();
    const otherSource =
      record("session_meta", {
        id: otherId,
        cwd: "/synthetic/read-only-repository",
        source: "cli",
        originator: "codex_cli_rs",
      }) +
      record("event_msg", {
        type: "user_message",
        message: "只读分析合成会话",
        images: [],
        local_images: [],
      });
    const otherPath = path.join(
      directory,
      `rollout-2026-09-17T08-00-00-${otherId}.jsonl`,
    );
    await fs.writeFile(otherPath, otherSource);
    await fs.utimes(sourcePath, new Date(), new Date(Date.now() + 60000));
    let running = await start(home, env);
    const target = await launch(launcher, running.url, width);
    await wait(
      target,
      "document.querySelectorAll('.qt-card').length===12&&document.querySelectorAll('.session-item').length===2",
      "两个会话与12个问题已加载",
    );
    await evaluate(
      target,
      "(()=>{window.__featureErrors=[];window.addEventListener('error',e=>window.__featureErrors.push(e.message));window.addEventListener('unhandledrejection',e=>window.__featureErrors.push(String(e.reason)));return true})()",
    );
    const listing = await api(running.url, "/api/sessions");
    const sessions = Array.isArray(listing.body)
      ? listing.body
      : listing.body.sessions;
    assert.ok(Array.isArray(sessions), "会话API可读取");
    const current = sessions.find((session) => session.id === id);
    assert.ok(current, "合成会话存在");
    const favoritePath = `/api/session/${current.key}/favorite`;
    assert.equal(
      (await api(running.url, favoritePath, { favorite: true }, false)).status,
      403,
      "收藏写入必须带token",
    );
    await wait(
      target,
      "document.querySelectorAll('.qt-card-commit').length===2&&document.querySelectorAll('.qt-compaction-marker').length===2",
      "提交与可关联压缩标识",
    );

    await click(target, '[data-question-id="q2"]');
    await pause(400);
    const selected = await view(target);
    assert.equal(selected.selected, "q2");
    await buttonText(target, ".layout-controls", "横向");
    await wait(
      target,
      "document.querySelector('.layout-controls button:nth-child(2)')?.getAttribute('aria-pressed')==='true'",
      "横向已生效",
    );
    await pause(400);
    assert.equal((await view(target)).selected, "q2", "切换方向保留选择");
    const loose = await coordinates(target);
    await buttonText(target, ".layout-controls", "舒适");
    await wait(
      target,
      "document.querySelectorAll('.qt-card.is-compact').length===12",
      "紧凑已生效",
    );
    await pause(400);
    assert.equal((await view(target)).selected, "q2", "切换密度保留选择");
    assert.notDeepEqual(
      await coordinates(target),
      loose,
      "密度改变坐标而非缩小整个画布",
    );
    await picture(target, root, "compact-selected.png");
    console.log(
      "所选节点几何",
      await evaluate(
        target,
        "JSON.stringify({node:document.querySelector('.qt-card.is-selected')?.getBoundingClientRect().toJSON(),canvas:document.querySelector('.qt-canvas')?.getBoundingClientRect().toJSON(),detail:document.querySelector('.detail-panel.has-selection')?.getBoundingClientRect().toJSON()})",
      ),
    );
    assert.equal(
      await evaluate(
        target,
        `(()=>{const n=document.querySelector('.qt-card.is-selected').getBoundingClientRect();const c=document.querySelector('.qt-canvas').getBoundingClientRect();const p=document.querySelector('.detail-panel.has-selection')?.getBoundingClientRect();const x=n.left+n.width/2,y=n.top+n.height/2;return x>=c.left&&x<=c.right&&y>=c.top&&y<=c.bottom&&(!p||x<p.left||x>p.right||y<p.top||y>p.bottom)})()`,
      ),
      true,
      "布局切换后所选问题位于详情未遮挡区域",
    );
    assert.equal(
      await evaluate(
        target,
        "document.querySelectorAll('.react-flow__edge').length",
      ),
      11,
      "布局保留真实11条边",
    );

    const beforeFavorite = await view(target);
    await evaluate(
      target,
      "(()=>{window.__edgeFrames=new Promise(resolve=>{const frames=[],until=performance.now()+800;const sample=()=>{frames.push(document.querySelectorAll('.react-flow__edge').length);if(performance.now()<until)requestAnimationFrame(sample);else resolve(frames)};requestAnimationFrame(sample)});return true})()",
    );
    await click(target, '[data-question-id="q4"] .qt-card-favorite');
    await wait(
      target,
      "!!document.querySelector('[data-question-id=\"q4\"].is-favorite')",
      "问题收藏已保存",
    );
    await pause(350);
    assert.deepEqual(
      await view(target),
      beforeFavorite,
      "收藏其他问题不会选择它或移动视口",
    );
    const edgeFrames = await evaluate(target, "window.__edgeFrames");
    assert.ok(
      edgeFrames.length > 1 && edgeFrames.every((count) => count === 11),
      "收藏与提示刷新不让连线闪烁消失",
    );
    const keyboard = await evaluate(
      target,
      "(()=>{const b=document.querySelector('[data-question-id=\"q4\"] .qt-card-favorite');b.focus({preventScroll:true});const e=new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true});b.dispatchEvent(e);b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true,cancelable:true}));return !e.defaultPrevented})()",
    );
    assert.equal(keyboard, true, "星标Enter不被节点抢占");
    await pause(350);
    assert.deepEqual(
      await view(target),
      beforeFavorite,
      "星标Enter及双击不触发节点查看或聚焦",
    );
    await click(target, '[aria-label="收藏当前会话"]');
    await wait(
      target,
      "document.querySelector('[aria-label=\"收藏当前会话\"]')?.getAttribute('aria-pressed')==='true'",
      "会话收藏已保存",
    );
    assert.equal(
      hash(await fs.readFile(sourcePath)),
      hash(source),
      "收藏不修改rollout字节",
    );

    await click(target, '[data-question-id="q2"] .qt-card-commit');
    await wait(
      target,
      "!!document.querySelector('.event-detail')",
      "提交徽标打开详情",
    );
    let detail = await evaluate(
      target,
      "document.querySelector('.event-detail').innerText",
    );
    for (const expected of [
      "/synthetic/canvas-repository",
      "main",
      "v3.1.0",
      "abc1234",
    ])
      assert.ok(detail.includes(expected), `提交详情包含 ${expected}`);
    await picture(target, root, "commit-light.png");
    await closeDetail(target);
    await click(target, '[data-question-id="q5"] .qt-card-commit');
    await wait(
      target,
      "document.querySelector('.event-detail')?.innerText.includes('def5678')",
      "第二提交详情",
    );
    detail = await evaluate(
      target,
      "document.querySelector('.event-detail').innerText",
    );
    assert.ok(
      detail.includes("未标记版本") && detail.includes("feature/canvas"),
      "未标记版本不伪造版本号",
    );
    await closeDetail(target);
    for (const [title, trigger] of [
      ["自动压缩", "自动"],
      ["手动压缩", "手动"],
      ["压缩 · 触发方式未记录", "未记录"],
    ]) {
      await eventDetail(target, title);
      detail = await evaluate(
        target,
        "document.querySelector('.event-detail').innerText",
      );
      assert.ok(
        detail.includes(title) && detail.includes(trigger),
        "压缩触发方式忠于记录",
      );
      if (trigger === "未记录") {
        assert.ok(detail.includes("Q12"), "末尾无边压缩仍可从事件列表查看");
        await picture(target, root, "compaction-light.png");
      }
      await closeDetail(target);
    }

    await sidebar(target);
    await buttonText(target, ".session-filters", "收藏");
    await wait(
      target,
      "document.querySelectorAll('.session-item').length===1",
      "收藏会话筛选",
    );
    await buttonText(target, ".session-filters", "有提交");
    await wait(
      target,
      "document.querySelectorAll('.session-item').length===1",
      "有提交会话筛选",
    );
    assert.ok(
      await evaluate(
        target,
        "document.querySelector('.session-item').innerText.includes('画布整理合成会话')",
      ),
    );
    await buttonText(target, ".session-filters", "全部");
    await closeSidebar(target);

    await click(target, '[data-question-id="q2"]');
    await pause(400);
    const beforeAppend = await view(target);
    const beforePositions = await coordinates(target);
    source += question(13);
    await fs.appendFile(sourcePath, question(13));
    await wait(
      target,
      "document.querySelectorAll('.qt-card').length===13",
      "增量追加已到达",
    );
    await pause(350);
    assert.deepEqual(
      await view(target),
      beforeAppend,
      "追加保留历史选择与视口",
    );
    const afterPositions = await coordinates(target);
    for (const [nodeId, position] of Object.entries(beforePositions))
      assert.equal(afterPositions[nodeId], position, "追加不重排历史节点");
    await closeDetail(target);
    await evaluate(
      target,
      "(()=>{const e=document.querySelector('select[aria-label=\"主题\"]');e.value='dark';e.dispatchEvent(new Event('change',{bubbles:true}));return true})()",
    );
    await pause(400);
    await picture(target, root, "canvas-dark.png");
    await evaluate(
      target,
      "(()=>{const e=document.querySelector('select[aria-label=\"主题\"]');e.value='light';e.dispatchEvent(new Event('change',{bubbles:true}));return true})()",
    );
    await pause(250);
    await picture(target, root, "canvas-light.png");

    assert.deepEqual(
      await evaluate(target, "window.__featureErrors"),
      [],
      "浏览器交互无运行时异常",
    );
    await cdp(
      `/navigate?target=${target}&url=${encodeURIComponent(running.url)}`,
    );
    await wait(
      target,
      "document.querySelectorAll('.qt-card').length===13&&!!document.querySelector('[data-question-id=\"q4\"].is-favorite')",
      "刷新后问题收藏保持",
    );
    assert.equal(
      await evaluate(
        target,
        "document.querySelector('[aria-label=\"收藏当前会话\"]').getAttribute('aria-pressed')",
      ),
      "true",
      "刷新后会话收藏保持",
    );
    assert.equal(
      await evaluate(
        target,
        "document.querySelectorAll('.qt-card.is-compact').length",
      ),
      13,
      "刷新后布局偏好保持",
    );
    const oldOrigin = new URL(running.url).origin;
    await stop(running.service);
    running = await start(home, env);
    assert.notEqual(new URL(running.url).origin, oldOrigin, "新服务采用新端口");
    await cdp(
      `/navigate?target=${target}&url=${encodeURIComponent(running.url)}`,
    );
    await wait(
      target,
      "document.querySelectorAll('.qt-card').length===13&&!!document.querySelector('[data-question-id=\"q4\"].is-favorite')",
      "换端口后问题收藏由服务端恢复",
    );
    assert.equal(
      await evaluate(
        target,
        "document.querySelector('[aria-label=\"收藏当前会话\"]').getAttribute('aria-pressed')",
      ),
      "true",
      "换端口后会话收藏保持",
    );
    assert.equal(
      hash(await fs.readFile(sourcePath)),
      hash(source),
      "除合成追加外源文件未变",
    );
    assert.equal(
      hash(await fs.readFile(otherPath)),
      hash(otherSource),
      "无关合成会话未变",
    );
    assert.ok(
      await evaluate(
        target,
        "document.documentElement.scrollWidth<=innerWidth",
      ),
      "窄屏无横向页面溢出",
    );
    await stop(running.service);
    await cdp(`/close?target=${target}`);
    tabs.delete(target);
    console.log(`功能验收 ${width}px 全部通过：${root}`);
  }
} finally {
  for (const service of services) await stop(service);
  for (const target of tabs)
    await cdp(`/close?target=${target}`).catch(() => {});
  console.log(`合成验收材料保留：${roots.join("、")}`);
}
