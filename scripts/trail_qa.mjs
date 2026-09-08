// Question Trail 的真实 Chrome 验收；仅经 web-access Proxy 操作自建窗口。
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";

const execute = promisify(execFile);

const binary = path.resolve(process.argv[2] || "target/release/codex-nav");
const fixture = await fs.mkdtemp(path.join(os.tmpdir(), "codex-trail-qa-"));
const home = path.join(fixture, "codex");
const dir = path.join(home, "sessions");
await fs.mkdir(dir, { recursive: true });
const record = (type, payload, timestamp = "2026-09-08T00:12:00Z") =>
  JSON.stringify({ type, payload, timestamp }) + "\n";
const event = (payload) => record("event_msg", payload);
const meta = (id, project) =>
  record("session_meta", { id, cwd: `/synthetic/${project}`, source: "cli" });
const questions = [
  "怎样找回长对话里那些重要的问题？",
  "可以把问题串成一张可交互的地图吗？",
  "点击问题时，能同时看到它的完整原文吗？",
  "另一条方向：如何确保会话文件始终只读？",
  "搜索和复制功能需要怎样保护隐私？",
  "新问题出现时，怎样保留我正在回看的位置？",
];
function turn(index, title = questions[index - 1], parent) {
  const id = `turn-${index}`;
  return (
    record("turn_context", {
      turn_id: id,
      ...(parent ? { parent_turn_id: `turn-${parent}` } : {}),
    }) +
    event({
      type: "user_message",
      message: `${title}\n\n${index === 2 ? "希望问题不再淹没在几百屏输出里。每张卡片保留真实的问题，点击即可回到原文，了解前后是怎样继续提问的。\n\n连线只表示已经记录的顺序与分支，不要替我推断因果，也不要调用 AI。" : "这是用于产品验收的合成会话，不包含真实用户记录。"}${index === 5 ? '\n<script>window.__unsafeTrail=true</script>\n<img src="https://invalid.example.test/tracker.png">' : ""}`,
    }) +
    record("response_item", {
      type: "reasoning",
      summary: [{ text: "PRIVATE_REASONING_DO_NOT_DISPLAY" }],
    }) +
    record("response_item", {
      type: "function_call",
      name: "inspect_local_file",
      call_id: `call-${index}`,
      arguments: JSON.stringify({ path: "synthetic-example.rs" }),
    }) +
    event({
      type: "task_complete",
      turn_id: id,
      last_agent_message: `第 ${index} 个问题的最终回复：可以使用本地只读记录来实现。\n\n不需要模型调用，也不会上传对话。${index === 5 ? "\n<script>window.__unsafeTrail=true</script>\n![remote](https://invalid.example.test/tracker.png)" : ""}`,
    })
  );
}
const rollouts = new Map([
  [
    "main",
    meta("trail-qa-main", "question-map") +
      [null, 1, 1, 2, 2, 3]
        .map((parent, i) => turn(i + 1, undefined, parent))
        .join(""),
  ],
  [
    "other",
    meta("trail-qa-other", "sidecar-notes") +
      turn(1, "Sidecar 独立入口怎样部署到另一台设备？"),
  ],
  ["empty", meta("trail-qa-empty", "empty-session")],
  [
    "linear",
    meta("trail-qa-linear", "linear-1000") +
      Array.from({ length: 1000 }, (_, index) =>
        turn(
          index + 1,
          `MAINLINE_${index + 1}：逐步回看第 ${index + 1} 个本地问题`,
        ),
      ).join(""),
  ],
  [
    "malformed",
    meta("trail-qa-malformed", "degraded-session") +
      turn(1, "损坏记录之后仍能看到可靠的问题吗？") +
      "{broken json\n" +
      record("future_schema", { type: "unknown_future_event" }),
  ],
]);
const files = new Map();
for (const [name, text] of rollouts) {
  const file = path.join(dir, `rollout-${name}.jsonl`);
  files.set(name, file);
  await fs.writeFile(file, text);
}
await fs.utimes(files.get("main"), new Date(), new Date(Date.now() + 60000));
const child = spawn(
  binary,
  [
    ...(path.basename(binary) === "codex-trail" ? [] : ["--web"]),
    "--port",
    "0",
    "--no-open",
    "--codex-home",
    home,
  ],
  {
    env: { ...process.env, XDG_CONFIG_HOME: path.join(fixture, "config") },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
const services = new Set([child]);
let childError = "";
child.stderr.on("data", (chunk) => {
  childError += chunk;
});
const tabs = new Set();
const appended = [];
async function cdp(endpoint, body) {
  const response = await fetch(`http://localhost:3456${endpoint}`, {
    ...(body === undefined ? {} : { method: "POST", body }),
    signal: AbortSignal.timeout(25000),
  });
  const result = await response.json();
  if (!response.ok || result.error)
    throw new Error(`CDP ${endpoint.split("?")[0]}: ${JSON.stringify(result)}`);
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
    `(async()=>{const until=Date.now()+18000;while(Date.now()<until){if(${condition})return true;await new Promise(r=>setTimeout(r,100))}return false})()`,
  );
  if (!passed)
    throw new Error(
      `${label}: ${JSON.stringify(await evaluate(target, "({text:document.body.innerText.slice(-2200),width:innerWidth,height:innerHeight})"))}`,
    );
}
const click = (target, selector) => cdp(`/click?target=${target}`, selector);
const input = (target, selector, value) =>
  evaluate(
    target,
    `(()=>{const e=document.querySelector(${JSON.stringify(selector)});Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`,
  );
const key = (target, value, selector = "body", modifiers = {}) =>
  evaluate(
    target,
    `(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.focus({preventScroll:true});e.dispatchEvent(new KeyboardEvent('keydown',{key:${JSON.stringify(value)},bubbles:true,cancelable:true,...${JSON.stringify(modifiers)}}));return true})()`,
  );
const selected = (index) =>
  `!!document.querySelector('[data-question-id="q${index}"].is-selected')&&document.querySelector('.question-heading h3')?.textContent.startsWith('${index}.')&&Number.isFinite(parseFloat(document.querySelector('[aria-label="缩放比例"]').textContent))`;
const settled = (target) =>
  evaluate(target, "new Promise(r=>setTimeout(()=>r(true),400))");
async function setTheme(target, mode) {
  await evaluate(
    target,
    `(()=>{const e=document.querySelector('select[aria-label="主题"]');e.value=${JSON.stringify(mode)};e.dispatchEvent(new Event('change',{bubbles:true}));return true})()`,
  );
  await wait(
    target,
    `document.querySelector('select[aria-label="主题"]').value===${JSON.stringify(mode)}&&document.documentElement.dataset.theme===${mode === "system" ? "(matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light')" : JSON.stringify(mode)}&&localStorage.getItem('questionTrail.theme')===${JSON.stringify(mode)}`,
    `主题 ${mode} 已应用并保存`,
  );
  await settled(target);
}
async function assertDarkSurfaces(target, selectors) {
  const colors = await evaluate(
    target,
    `(${JSON.stringify(selectors)}).map(selector=>{const e=document.querySelector(selector);if(!e)throw new Error('Missing dark surface '+selector);const style=getComputedStyle(e);return {selector,background:style.backgroundColor,color:style.color}})`,
  );
  for (const { selector, background, color } of colors) {
    const rgb = background.match(/[\d.]+/g)?.map(Number);
    assert.ok(
      rgb &&
        rgb.length >= 3 &&
        (rgb.length === 3 || rgb[3] === 1) &&
        rgb.slice(0, 3).every((channel) => channel < 120),
      `深色表面不残留亮底 ${selector}: ${background}`,
    );
    const foreground = color.match(/[\d.]+/g)?.map(Number);
    assert.ok(
      foreground && foreground.slice(0, 3).some((channel) => channel > 130),
      `深色文字可读 ${selector}: ${color}`,
    );
  }
}
async function reloadAndCheckTheme(target, mode) {
  const url = await evaluate(target, "location.href");
  await cdp(`/navigate?target=${target}&url=${encodeURIComponent(url)}`);
  await wait(
    target,
    `document.querySelectorAll('.qt-card').length>=6&&document.querySelector('select[aria-label="主题"]')?.value===${JSON.stringify(mode)}&&document.documentElement.dataset.theme===${mode === "system" ? "(matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light')" : JSON.stringify(mode)}`,
    `刷新后 ${mode} 主题与会话恢复`,
  );
}
async function choose(target, text) {
  await evaluate(
    target,
    `(()=>{const e=[...document.querySelectorAll('.session-item')].find(e=>e.textContent.includes(${JSON.stringify(text)}));if(!e)throw new Error('session not found');e.click();return true})()`,
  );
}
async function screenshot(target, name) {
  const file = path.join(fixture, name);
  const extensions = await evaluate(
    target,
    `(()=>{
    const marked=/immersive[-_ ]?translat|chrome-extension:|translate[-_]?extension|kiss[-_]?translat/i;
    const hidden=[], inspected=[];
    const walk=(root,inheritedProof=null)=>{for(const element of root.children||[]){
      if(element.id==='root')continue;
      const identity=[element.tagName,element.id,typeof element.className==='string'?element.className:'',element.getAttribute('src')||''].join(' ');
      const shadow=element.shadowRoot;
      const proof=(identity+' '+(shadow?.innerHTML||'')).match(marked)?.[0]||inheritedProof;
      const fixed=getComputedStyle(element).position==='fixed';
      if(fixed||proof)inspected.push({tag:element.tagName,id:element.id,identity:identity.slice(0,180),fixed,openShadow:!!shadow,proof});
      if(fixed&&proof){hidden.push({element,style:element.getAttribute('style')});element.style.setProperty('display','none','important');}
      if(shadow)walk(shadow,proof);
      walk(element,proof);
    }};
    walk(document.documentElement);
    window.__qaHiddenExtensions=hidden;
    return {hidden:hidden.length,inspected:inspected.slice(0,30)};
  })()`,
  );
  console.log(`截图外部浮层检查 ${name}：${JSON.stringify(extensions)}`);
  try {
    await cdp(`/screenshot?target=${target}&file=${encodeURIComponent(file)}`);
  } finally {
    await evaluate(
      target,
      "(()=>{for(const {element,style} of window.__qaHiddenExtensions||[]){if(style===null)element.removeAttribute('style');else element.setAttribute('style',style)}delete window.__qaHiddenExtensions;return true})()",
    );
  }
  return file;
}
async function resizeOwnedWindow(target, width, height) {
  const title = `QuestionTrailQA-${process.pid}-${width}-${Date.now()}`;
  const previous = await evaluate(
    target,
    `(()=>{const previous=document.title;document.title=${JSON.stringify(title)};return previous})()`,
  );
  try {
    await settled(target);
    const { stdout } = await execute(
      "xdotool",
      ["search", "--onlyvisible", "--name", `^${title}( - Google Chrome)?$`],
      { timeout: 3000 },
    );
    const ids = stdout.trim().split(/\s+/).filter(Boolean);
    if (ids.length !== 1 || !/^\d+$/.test(ids[0])) return;
    const { stdout: actualTitle } = await execute(
      "xdotool",
      ["getwindowname", ids[0]],
      { timeout: 3000 },
    );
    if (![title, `${title} - Google Chrome`].includes(actualTitle.trim()))
      return;
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
        String(height + border.height),
      ],
      { timeout: 3000 },
    );
    await settled(target);
  } catch {
    // Window managers may ignore requested geometry. Report the actual viewport below.
  } finally {
    await evaluate(
      target,
      `(()=>{document.title=${JSON.stringify(previous)};return true})()`,
    );
  }
}
async function drag(target, selector, dx, dy) {
  return evaluate(
    target,
    `(async()=>{const e=document.querySelector(${JSON.stringify(selector)}),r=e.getBoundingClientRect(),x=r.left+r.width/2,y=r.top+r.height/2;const init={bubbles:true,cancelable:true,view:window,button:0,buttons:1,clientX:x,clientY:y};e.dispatchEvent(new MouseEvent('mousedown',init));for(let step=1;step<=8;step++){await new Promise(r=>setTimeout(r,40));window.dispatchEvent(new MouseEvent('mousemove',{...init,clientX:x+${dx}*step/8,clientY:y+${dy}*step/8}));}window.dispatchEvent(new MouseEvent('mouseup',{...init,buttons:0,clientX:x+${dx},clientY:y+${dy}}));return true})()`,
  );
}

try {
  const url = await new Promise((resolve, reject) => {
    let output = "";
    const timeout = setTimeout(
      () => reject(new Error(`启动超时：${childError}`)),
      12000,
    );
    child.once("error", reject);
    child.once("exit", () => reject(new Error(`服务退出：${childError}`)));
    child.stdout.on("data", (chunk) => {
      output += chunk;
      const match = output.match(
        /http:\/\/127\.0\.0\.1:\d+\/(?:trail\/)?#token=[a-zA-Z0-9_-]+/,
      );
      if (match) {
        clearTimeout(timeout);
        resolve(match[0]);
      }
    });
  });
  const launcher = (await cdp("/new?url=about%3Ablank")).targetId;
  tabs.add(launcher);
  for (const width of [1672, 1000, 390]) {
    const height = width === 1672 ? 941 : 850;
    const qaUrl = url.replace("/#", `/?trailqa=${width}#`);
    await evaluate(
      launcher,
      `(()=>{document.body.replaceChildren();const b=document.createElement('button');b.id='launch';b.textContent='打开问题画布验收';b.onclick=()=>window.open(${JSON.stringify(qaUrl)},'trail-qa-${width}-${Date.now()}','popup,width=${width},height=${height}');document.body.append(b);return true})()`,
    );
    await cdp(`/clickAt?target=${launcher}`, "#launch");
    let target;
    for (let attempt = 0; attempt < 40 && !target; attempt++) {
      target = (await cdp("/targets")).find((tab) =>
        tab.url.startsWith(qaUrl.split("#")[0]),
      )?.targetId;
      if (!target)
        await new Promise((resolve) => {
          setTimeout(resolve, 100);
        });
    }
    assert.ok(target, "独立真实窗口");
    tabs.add(target);
    await evaluate(
      target,
      `(()=>{window.resizeTo(${width}+outerWidth-innerWidth,${height}+outerHeight-innerHeight);return true})()`,
    );
    await settled(target);
    await resizeOwnedWindow(target, width, height);
    await wait(
      target,
      "document.querySelectorAll('.session-item').length===5&&document.querySelectorAll('.qt-card').length>=6",
      "自动发现并打开最近会话",
    );
    const actualWidth = await evaluate(target, "innerWidth");
    assert.ok(
      width === 390
        ? actualWidth === 390
        : width === 1000
          ? actualWidth >= 900 && actualWidth < 1280
          : actualWidth >= 1280,
      "真实响应式视口",
    );
    console.log(`窗口请求 ${width}px，实际验收 ${actualWidth}px`);
    await setTheme(target, width === 390 ? "dark" : "light");
    assert.ok(
      await evaluate(
        target,
        "document.querySelectorAll('.qt-connection.is-branch').length===4",
      ),
      "真实持久化分支",
    );
    await click(target, '[data-question-id="q2"]');
    await wait(target, selected(2), "点击 Q2 更新详情");
    if (width === 1000) {
      await settled(target);
      await wait(
        target,
        "(()=>{const node=document.querySelector('[data-question-id=q2]').getBoundingClientRect(),sheet=document.querySelector('.detail-panel').getBoundingClientRect(),stage=document.querySelector('.canvas-stage').getBoundingClientRect(),center=(node.left+node.right)/2;return center>=stage.left&&center<sheet.left&&Number.isFinite(node.left)})()",
        "中屏右侧详情不遮挡选中问题",
      );
      await screenshot(target, `trail-${actualWidth}.png`);
      await key(target, "Escape", ".detail-panel");
      await key(target, "f");
      await settled(target);
      assert.ok(
        await evaluate(
          target,
          "Number.isFinite(parseFloat(document.querySelector('[aria-label=缩放比例]').textContent))",
        ),
        "中屏fit正常",
      );
      console.log(
        `PASS Question Trail ${actualWidth}px：中屏drawer/选中可见/关闭与fit`,
      );
      await cdp(`/close?target=${target}`);
      tabs.delete(target);
      continue;
    }
    await wait(
      target,
      "document.querySelector('.prompt-text')?.textContent.includes('几百屏输出')",
      "问题全文载入",
    );
    assert.ok(
      await evaluate(
        target,
        "document.querySelector('.branch-relations')?.textContent.includes('直接分支（2）')",
      ),
      "直接分支关系",
    );
    await settled(target);
    if (width === 1672)
      console.log(
        `初始桌面截图：${await screenshot(target, "trail-first-desktop.png")}`,
      );
    await click(target, ".detail-navigation button:last-child");
    await wait(target, selected(3), "下一个问题");
    await key(target, "ArrowUp", ".detail-panel");
    await wait(target, selected(2), "上方向键上一问题");
    await key(target, "ArrowDown", ".detail-panel");
    await wait(target, selected(3), "下方向键下一问题");
    await click(target, ".detail-navigation button:first-child");
    await wait(target, selected(2), "上一个按钮");
    await click(
      target,
      ".canvas-toolbar .toolbar-group:first-child button:last-child",
    );
    await wait(
      target,
      "document.querySelectorAll('.react-flow__node[style*=\"0.22\"]').length>0",
      "聚焦路径淡化无关节点",
    );
    await click(
      target,
      ".canvas-toolbar .toolbar-group:first-child button:first-child",
    );
    await settled(target);
    const initialZoom = await evaluate(
      target,
      "document.querySelector('[aria-label=缩放比例]').textContent",
    );
    await click(target, '[aria-label="放大"]');
    await wait(
      target,
      `document.querySelector('[aria-label="缩放比例"]').textContent!==${JSON.stringify(initialZoom)}`,
      "缩放控件",
    );
    await key(target, "f");
    await settled(target);
    await click(target, '[aria-label="画布全屏"]');
    assert.ok(
      await evaluate(
        target,
        "document.querySelector('.app-shell').classList.contains('canvas-expanded')",
      ),
      "画布全屏",
    );
    await click(target, '[aria-label="退出画布全屏"]');
    await settled(target);
    const minimap = await evaluate(
      target,
      "getComputedStyle(document.querySelector('.qt-minimap')).display",
    );
    assert.equal(minimap === "none", width < 900, "小地图响应式显示");
    assert.deepEqual(
      await evaluate(
        target,
        "(()=>{const svg=document.querySelector('.qt-minimap svg');return {width:Number(svg.getAttribute('width')),height:Number(svg.getAttribute('height'))}})()",
      ),
      { width: 160, height: 92 },
      "小地图SVG与容器尺寸一致",
    );
    if (width === 1672) {
      await evaluate(
        target,
        "(()=>{document.querySelector('.react-flow__node[data-id=q4]').dispatchEvent(new MouseEvent('mouseover',{bubbles:true,view:window}));return true})()",
      );
      await wait(
        target,
        "document.querySelector('.qt-tooltip')?.textContent.includes('下游')",
        "hover 延迟提示和结构信息",
      );
      await evaluate(
        target,
        "(()=>{document.querySelector('.react-flow__node[data-id=q4]').dispatchEvent(new MouseEvent('mouseout',{bubbles:true,view:window,relatedTarget:document.body}));return true})()",
      );
      await evaluate(
        target,
        "(()=>{document.querySelector('.react-flow__edge[data-id=\"q2-q4\"]').dispatchEvent(new MouseEvent('mouseover',{bubbles:true,view:window}));return true})()",
      );
      await wait(
        target,
        "document.querySelector('.qt-edge-label')?.textContent==='Q2 → Q4'",
        "连线 hover 显示实际端点",
      );
      await evaluate(
        target,
        "(()=>{document.querySelector('.react-flow__edge[data-id=\"q2-q4\"]').dispatchEvent(new MouseEvent('mouseout',{bubbles:true,view:window,relatedTarget:document.body}));return true})()",
      );
      const position = await evaluate(
        target,
        "document.querySelector('.react-flow__node[data-id=q4]').style.transform",
      );
      await drag(target, '.react-flow__node[data-id="q4"]', 60, 25);
      await wait(
        target,
        `document.querySelector('.react-flow__node[data-id=q4]').style.transform!==${JSON.stringify(position)}`,
        "节点可拖动",
      );
      await evaluate(
        target,
        "(()=>{document.querySelector('.react-flow__node[data-id=q2]').dispatchEvent(new MouseEvent('dblclick',{bubbles:true,view:window}));return true})()",
      );
      await wait(
        target,
        "!!document.querySelector('.qt-return-map')",
        "双击局部聚焦",
      );
      await click(target, ".qt-return-map");
      await settled(target);
      const viewport = await evaluate(
        target,
        "document.querySelector('.react-flow__viewport').style.transform",
      );
      await drag(target, ".react-flow__pane", 40, 25);
      await wait(
        target,
        `document.querySelector('.react-flow__viewport').style.transform!==${JSON.stringify(viewport)}`,
        "背景可平移",
      );
      await key(target, "f");
      await settled(target);
      const shot = await screenshot(target, `trail-${actualWidth}.png`);
      await fs.mkdir(path.resolve("docs/images"), { recursive: true });
      await fs.copyFile(shot, path.resolve("docs/images/question-trail.png"));
      await reloadAndCheckTheme(target, "light");
      await click(target, '[data-question-id="q2"]');
      await wait(target, selected(2), "刷新后重新选择 Q2");
      await settled(target);
      const themeViewport = await evaluate(
        target,
        "document.querySelector('.react-flow__viewport').style.transform",
      );
      await setTheme(target, "dark");
      assert.ok(await evaluate(target, selected(2)), "切换主题保持选中问题");
      assert.equal(
        await evaluate(
          target,
          "document.querySelector('.react-flow__viewport').style.transform",
        ),
        themeViewport,
        "切换主题保持历史视口",
      );
      await assertDarkSurfaces(target, [
        ".session-sidebar",
        ".topbar",
        ".qt-canvas",
        ".react-flow",
        ".react-flow__background",
        ".qt-card",
        ".detail-panel",
        ".qt-minimap",
        "select[aria-label=主题]",
      ]);
      assert.equal(
        await evaluate(
          target,
          "getComputedStyle(document.querySelector('.react-flow__background')).backgroundColor",
        ),
        await evaluate(
          target,
          "getComputedStyle(document.querySelector('.qt-canvas')).backgroundColor",
        ),
        "React Flow 背景 SVG 沿用主题背景，不回退库默认黑色",
      );
      await key(target, "f");
      await settled(target);
      const darkShot = await screenshot(
        target,
        `trail-dark-${actualWidth}.png`,
      );
      await fs.copyFile(
        darkShot,
        path.resolve("docs/images/question-trail-dark.png"),
      );
      await key(target, "k", "body", { ctrlKey: true });
      await wait(
        target,
        "!!document.querySelector('.search-dialog[open]')",
        "深色全局搜索",
      );
      await input(target, '[aria-label="搜索问题原文"]', "Sidecar");
      await wait(
        target,
        "!!document.querySelector('.search-result')",
        "深色搜索结果",
      );
      await assertDarkSurfaces(target, [".search-dialog"]);
      await screenshot(target, "trail-dark-search.png");
      await key(target, "Escape", '[aria-label="搜索问题原文"]');
      await reloadAndCheckTheme(target, "dark");
      await setTheme(target, "system");
      await reloadAndCheckTheme(target, "system");
      await setTheme(target, "light");
      console.log(
        "PASS 主题：浅色/深色/跟随系统、三态刷新保存、卡片/面板/小地图/搜索无亮底",
      );
      await evaluate(
        target,
        "(()=>{window.__linearStart=performance.now();return true})()",
      );
      await choose(target, "linear-1000");
      await wait(
        target,
        "document.querySelector('.canvas-heading p')?.textContent.startsWith('1000 个问题')&&document.querySelectorAll('.qt-card').length>=6",
        "1000问题主线加载",
      );
      await settled(target);
      const linear = await evaluate(
        target,
        "(()=>{const stage=document.querySelector('.canvas-stage').getBoundingClientRect();return {ms:Math.round(performance.now()-window.__linearStart),dom:document.querySelectorAll('.qt-card').length,zoom:parseFloat(document.querySelector('[aria-label=缩放比例]').textContent),firstSix:[1,2,3,4,5,6].every(i=>{const n=document.querySelector('[data-question-id=q'+i+']');if(!n)return false;const r=n.getBoundingClientRect();return r.top>=stage.top-2&&r.bottom<=stage.bottom+2&&r.left>=stage.left-2&&r.right<=stage.right+2&&r.width>=180})}})()",
      );
      assert.ok(linear.dom < 1000, "1000节点按视口虚拟化");
      assert.ok(
        linear.firstSix && Number.isFinite(linear.zoom),
        `首次六节点可见且可读 ${JSON.stringify(linear)}`,
      );
      await screenshot(target, `trail-linear-${actualWidth}.png`);
      const beforeMap = await evaluate(
        target,
        "document.querySelector('.react-flow__viewport').style.transform",
      );
      await evaluate(
        target,
        "(()=>{window.__minimapClicks=0;document.querySelector('.qt-minimap svg').addEventListener('click',()=>window.__minimapClicks++);return true})()",
      );
      await cdp(`/clickAt?target=${target}`, ".qt-minimap svg");
      await settled(target);
      const mapClick = await evaluate(
        target,
        "(()=>{const svg=document.querySelector('.qt-minimap svg'),r=svg.getBoundingClientRect(),hit=document.elementFromPoint(r.left+r.width/2,r.top+r.height/2);return {events:window.__minimapClicks,hit:hit?.tagName,cls:hit?.getAttribute('class'),svg:{width:r.width,height:r.height},viewport:document.querySelector('.react-flow__viewport').style.transform}})()",
      );
      console.log(`小地图点击诊断：${JSON.stringify(mapClick)}`);
      if (!mapClick.events) {
        // Background-window native input can be ignored by the desktop. Dispatch the
        // same coordinate-bearing DOM click without changing the application's state.
        await evaluate(
          target,
          "(()=>{const svg=document.querySelector('.qt-minimap svg'),r=svg.getBoundingClientRect();svg.dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true,view:window,clientX:r.left+r.width/2,clientY:r.top+r.height/2}));return true})()",
        );
      }
      await wait(
        target,
        `document.querySelector('.react-flow__viewport').style.transform!==${JSON.stringify(beforeMap)}`,
        "小地图点击定位长主线",
      );
      await key(target, "k", "body", { ctrlKey: true });
      await wait(
        target,
        "!!document.querySelector('.search-dialog[open]')",
        "长主线搜索入口",
      );
      await input(target, '[aria-label="搜索问题原文"]', "MAINLINE_1000");
      await wait(
        target,
        "document.querySelector('.search-result')?.textContent.includes('MAINLINE_1000')",
        "搜索最末问题",
      );
      await evaluate(
        target,
        "(()=>{window.__jumpStart=performance.now();return true})()",
      );
      await key(target, "Enter", '[aria-label="搜索问题原文"]');
      await wait(target, selected(1000), "搜索跳转Q1000并聚焦");
      await wait(
        target,
        "document.querySelector('.prompt-text')?.textContent.includes('MAINLINE_1000')",
        "Q1000原文准确",
      );
      const jumpMs = await evaluate(
        target,
        "Math.round(performance.now()-window.__jumpStart)",
      );
      await key(target, "f");
      await settled(target);
      assert.ok(
        await evaluate(
          target,
          "Number.isFinite(parseFloat(document.querySelector('[aria-label=缩放比例]').textContent))",
        ),
        "1000节点fit保持finite",
      );
      console.log(
        `PASS 1000问题主线：初次可读${linear.ms}ms，初始DOM${linear.dom}，搜索跳转${jumpMs}ms；小地图/fit/原文（含CDP等待）`,
      );
      await choose(target, "question-map");
      await wait(
        target,
        "document.querySelectorAll('.qt-card').length===6",
        "长主线返回六节点会话",
      );
    }
    await click(target, '[data-question-id="q5"]');
    await wait(target, selected(5), "打开安全测试问题");
    await click(target, ".raw-button");
    await wait(
      target,
      "document.querySelector('.answer-text')?.textContent.includes('第 5 个问题的最终回复')",
      "真实最终回复",
    );
    await click(target, ".activity-disclosure > summary");
    assert.ok(
      await evaluate(
        target,
        "document.querySelectorAll('.activity-item').length>0",
      ),
      "活动折叠摘要",
    );
    assert.deepEqual(
      await evaluate(
        target,
        "({unsafe:!!window.__unsafeTrail,hidden:document.body.innerText.includes('PRIVATE_REASONING_DO_NOT_DISPLAY'),images:document.querySelectorAll('.detail-panel img').length,external:performance.getEntriesByType('resource').filter(r=>/^https?:/.test(r.name)&&!r.name.startsWith(location.origin)).length,overflow:document.documentElement.scrollWidth>innerWidth+1})",
      ),
      { unsafe: false, hidden: false, images: 0, external: 0, overflow: false },
    );
    await evaluate(
      target,
      "(()=>{Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async(text)=>{window.__copiedTrail=text}}});return true})()",
    );
    await click(target, ".detail-actions > button:nth-child(2)");
    await wait(
      target,
      "window.__copiedTrail?.includes('搜索和复制')&&window.__copiedTrail.includes('<script>')",
      "复制问题原文而非回复",
    );
    await key(target, "Escape", ".detail-panel");
    await wait(
      target,
      "!document.querySelector('.detail-panel.has-selection')",
      "Escape 关闭详情",
    );
    await key(target, "k", "body", { ctrlKey: true });
    await wait(
      target,
      "!!document.querySelector('.search-dialog[open]')",
      "Ctrl K 打开全局搜索",
    );
    await input(target, '[aria-label="搜索问题原文"]', "完全没有这个关键词");
    await wait(
      target,
      "document.querySelector('.search-empty')?.textContent.includes('没有找到')",
      "搜索空态",
    );
    await click(target, '[aria-label="清除搜索"]');
    await wait(
      target,
      "document.querySelector('[aria-label=搜索问题原文]').value===''&&document.querySelectorAll('.search-result').length===0",
      "清空搜索",
    );
    await input(target, '[aria-label="搜索问题原文"]', "Sidecar");
    await wait(
      target,
      "document.querySelector('.search-result')?.textContent.includes('另一台设备')",
      "跨会话搜索结果",
    );
    await key(target, "Enter", '[aria-label="搜索问题原文"]');
    await wait(
      target,
      "document.querySelector('.prompt-text')?.textContent.includes('Sidecar 独立入口')&&!document.querySelector('.search-dialog')",
      "搜索跳转选中另一会话",
    );
    await choose(target, "degraded-session");
    await wait(
      target,
      "document.querySelector('.data-notice')?.textContent.includes('损坏')&&document.querySelectorAll('.qt-card').length===1",
      "坏行降级仍可阅读",
    );
    await choose(target, "empty-session");
    await wait(
      target,
      "document.querySelector('.canvas-empty')?.textContent.includes('还没有用户问题')",
      "无问题会话空态",
    );
    await choose(target, "question-map");
    await wait(
      target,
      "document.querySelectorAll('.qt-card').length>=6",
      "会话切换恢复",
    );
    await click(target, '[data-question-id="q2"]');
    await wait(target, selected(2), "选择历史问题");
    await settled(target);
    if (width === 390) {
      await wait(
        target,
        "(()=>{const node=document.querySelector('[data-question-id=q2]').getBoundingClientRect(),sheet=document.querySelector('.detail-panel').getBoundingClientRect(),stage=document.querySelector('.canvas-stage').getBoundingClientRect(),center=(node.top+node.bottom)/2;return center>=stage.top&&center<sheet.top})()",
        "移动详情打开后选中节点仍在可见画布内",
      );
      await wait(
        target,
        "!document.querySelector('.toast')",
        "临时提示消失后截图",
      );
      await screenshot(target, "trail-390.png");
      await assertDarkSurfaces(target, [
        ".topbar",
        ".qt-canvas",
        ".react-flow",
        ".react-flow__background",
        ".qt-card",
        ".detail-panel",
      ]);
      console.log("PASS 390px 深色主题：详情/卡片/画布可读，无横向溢出");
      await evaluate(
        target,
        "(()=>{window.__historyNode=document.querySelector('[data-question-id=q2]');window.__historyViewport=document.querySelector('.react-flow__viewport').style.transform;window.__historyPrompt=document.querySelector('.prompt-text');return true})()",
      );
      const newTurn = turn(7, "新问题：不打断历史视口的增量更新", 6);
      const split = newTurn.lastIndexOf("\n", newTurn.length - 2) + 1;
      const prefix = newTurn.slice(0, split);
      const tail = newTurn.slice(split);
      appended.push(prefix);
      await fs.appendFile(files.get("main"), prefix);
      await wait(
        target,
        "document.querySelector('.new-questions')?.textContent.includes('1 个新问题')",
        "SSE 新问题提示",
      );
      assert.ok(
        await evaluate(
          target,
          "window.__historyNode.isConnected&&window.__historyPrompt.isConnected&&document.querySelector('.react-flow__viewport').style.transform===window.__historyViewport&&document.querySelector('[data-question-id=q2]').classList.contains('is-selected')",
        ),
        "追加保留历史节点、DOM 和视口",
      );
      appended.push(tail.slice(0, -1));
      await fs.appendFile(files.get("main"), tail.slice(0, -1));
      await settled(target);
      assert.equal(
        await evaluate(
          target,
          "document.querySelectorAll('[data-question-id=q7]').length",
        ),
        1,
        "不完整尾行不重复问题",
      );
      appended.push("\n");
      await fs.appendFile(files.get("main"), "\n");
      await settled(target);
      assert.equal(
        await evaluate(
          target,
          "document.querySelectorAll('[data-question-id=q7]').length",
        ),
        1,
        "补全尾行恰好一个节点",
      );
      await click(target, ".new-questions");
      await wait(target, selected(7), "用户点击才跳最新");
      await wait(
        target,
        "!document.querySelector('.new-questions')",
        "新增计数已读清除",
      );
      await click(target, ".raw-button");
      await wait(
        target,
        "document.querySelector('.answer-text')?.textContent.includes('第 7 个问题的最终回复')",
        "尾行补全的最终回复可读",
      );
      const partialQuestion =
        record("turn_context", { turn_id: "turn-8" }) +
        event({
          type: "user_message",
          message: "第八个问题：不完整的输入行应等待换行",
        }).slice(0, -1);
      appended.push(partialQuestion);
      await fs.appendFile(files.get("main"), partialQuestion);
      await settled(target);
      assert.equal(
        await evaluate(target, "document.querySelectorAll('.qt-card').length"),
        7,
        "不完整用户输入不提前生成节点",
      );
      appended.push("\n");
      await fs.appendFile(files.get("main"), "\n");
      await wait(
        target,
        "document.querySelectorAll('.qt-card').length===8&&document.querySelector('.new-questions')?.textContent.includes('1 个新问题')",
        "补全用户行后精确新增一次",
      );
      await key(target, "Escape", ".detail-panel");
      await click(target, '[aria-label="打开会话列表"]');
      await wait(
        target,
        "document.querySelector('.app-shell').classList.contains('sidebar-open')",
        "移动会话模态栏",
      );
      await click(target, ".sidebar-scrim");
      await wait(
        target,
        "!document.querySelector('.app-shell').classList.contains('sidebar-open')",
        "关闭移动会话栏",
      );
    }
    console.log(
      `PASS Question Trail ${actualWidth}px：启动/真实分支/节点详情/前后跳转/搜索/缩放聚焦/原文/安全/空态/降级${width === 390 ? "/SSE追加与尾行保护" : "/hover/拖动/小地图"}`,
    );
    await cdp(`/close?target=${target}`);
    tabs.delete(target);
  }
  const staticHome = path.join(fixture, "static-codex");
  const staticDir = path.join(staticHome, "sessions");
  await fs.mkdir(staticDir, { recursive: true });
  const staticPath = path.join(staticDir, "rollout-target.jsonl");
  const latestPath = path.join(staticDir, "rollout-latest.jsonl");
  const staticText =
    meta("trail-static-target", "specified-session") +
    turn(1, "静态指定会话：应优先于最近会话打开");
  const latestText =
    meta("trail-static-latest", "latest-session") +
    turn(1, "最近会话：指定启动时不应自动打开这一条");
  await fs.writeFile(staticPath, staticText);
  await fs.writeFile(latestPath, latestText);
  await fs.utimes(latestPath, new Date(), new Date(Date.now() + 120000));
  const staticService = spawn(
    path.basename(binary) === "codex-trail"
      ? path.join(path.dirname(binary), "codex-nav")
      : binary,
    [
      "--web",
      "--port",
      "0",
      "--no-open",
      "--no-watch",
      "--codex-home",
      staticHome,
      "--session",
      "trail-static-target",
    ],
    {
      env: { ...process.env, XDG_CONFIG_HOME: path.join(fixture, "config") },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  services.add(staticService);
  let staticError = "";
  staticService.stderr.on("data", (chunk) => {
    staticError += chunk;
  });
  const staticUrl = await new Promise((resolve, reject) => {
    let output = "";
    const timeout = setTimeout(
      () => reject(new Error(`静态服务启动超时：${staticError}`)),
      12000,
    );
    staticService.once("error", reject);
    staticService.once("exit", () =>
      reject(new Error(`静态服务退出：${staticError}`)),
    );
    staticService.stdout.on("data", (chunk) => {
      output += chunk;
      const match = output.match(
        /http:\/\/127\.0\.0\.1:\d+\/(?:trail\/)?#token=[a-zA-Z0-9_-]+/,
      );
      if (match) {
        clearTimeout(timeout);
        resolve(match[0]);
      }
    });
  });
  const staticTab = (await cdp(`/new?url=${encodeURIComponent(staticUrl)}`))
    .targetId;
  tabs.add(staticTab);
  await wait(
    staticTab,
    "document.querySelectorAll('.qt-card').length===1&&document.querySelector('.canvas-heading h1')?.textContent.includes('静态指定会话')&&document.querySelector('.local-status')?.textContent.includes('手动刷新')",
    "--session 打开指定而非最新会话；--no-watch 显示手动刷新",
  );
  const staticAppend = turn(2, "手动刷新后才能显示的第二个问题");
  await fs.appendFile(staticPath, staticAppend);
  await evaluate(staticTab, "new Promise(r=>setTimeout(()=>r(true),2500))");
  assert.equal(
    await evaluate(staticTab, "document.querySelectorAll('.qt-card').length"),
    1,
    "--no-watch 不自动读取新增问题",
  );
  await click(staticTab, '[aria-label="刷新当前会话"]');
  await wait(
    staticTab,
    "document.querySelectorAll('.qt-card').length===2",
    "点击刷新才读取静态会话新增问题",
  );
  await click(staticTab, '[data-question-id="q2"]');
  await wait(
    staticTab,
    "document.querySelector('.prompt-text')?.textContent.includes('手动刷新后才能显示')",
    "刷新后的问题原文正确",
  );
  assert.equal(
    await fs.readFile(staticPath, "utf8"),
    staticText + staticAppend,
    "静态会话仅存在脚本预期追加",
  );
  assert.equal(
    await fs.readFile(latestPath, "utf8"),
    latestText,
    "最新会话保持只读",
  );
  await cdp(`/close?target=${staticTab}`);
  tabs.delete(staticTab);
  console.log(
    "PASS --session 指定会话、--no-watch 静态保护、手动刷新增量、只读校验",
  );
  const hash = (value) => createHash("sha256").update(value).digest("hex");
  for (const [name, text] of rollouts)
    assert.equal(
      hash(await fs.readFile(files.get(name))),
      hash(text + (name === "main" ? appended.join("") : "")),
      `${name} 仅存在脚本预期追加`,
    );
  console.log("PASS 合成记录只读校验；README 截图已更新");
} finally {
  for (const target of tabs)
    await cdp(`/close?target=${target}`).catch(() => {});
  for (const service of services) {
    if (service.exitCode === null) {
      service.kill("SIGINT");
      await Promise.race([
        once(service, "exit"),
        new Promise((resolve) => {
          setTimeout(resolve, 5000);
        }),
      ]);
      if (service.exitCode === null) service.kill("SIGTERM");
    }
  }
  console.log(`合成验收目录与截图保留：${fixture}`);
}
