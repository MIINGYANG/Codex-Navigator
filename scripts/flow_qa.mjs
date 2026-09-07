// 问题脉络真实浏览器验收；仅经 web-access CDP Proxy 操作自建标签页。
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const binary = path.resolve(process.argv[2] || "target/release/codex-nav");
const fixture = await fs.mkdtemp(path.join(os.tmpdir(), "codex-nav-flow-qa-"));
const codexHome = path.join(fixture, "codex");
await fs.mkdir(path.join(codexHome, "sessions"), { recursive: true });
const rollout = path.join(codexHome, "sessions", "rollout-flow-main.jsonl");
const other = path.join(codexHome, "sessions", "rollout-flow-other.jsonl");
const record = (type, payload) => JSON.stringify({ type, payload }) + "\n";
const event = (payload) => record("event_msg", payload);
function turn(index, prefix = "甲", complete = true) {
  const id = `${prefix}-${index}`;
  let result = event({ type: "task_started", turn_id: id });
  result += event({
    type: "user_message",
    message: `${prefix}问题 ${index}：${[1, 4].includes(index) ? "搜索间隙" : "独立验证"}\n${"这是一段合成的中文问题，验证 Unicode 🌱 与原文展开。\n".repeat(65)}`,
  });
  result += record("response_item", {
    type: "message",
    role: "assistant",
    phase: "commentary",
    content: [
      {
        type: "output_text",
        text: `${prefix}进展 ${index}：已记录的公开消息。`,
      },
    ],
  });
  for (let i = 0; i < (index === 0 ? 76 : 2); i++)
    result += record("response_item", {
      type: "function_call",
      name: "exec_command",
      call_id: `${id}-${i}`,
      arguments: JSON.stringify({ cmd: `echo ${prefix}-step-${index}-${i}` }),
    });
  result += record("response_item", {
    type: "function_call_output",
    call_id: `${id}-0`,
    output: "Process exited with code 1\nsynthetic warning",
  });
  if (complete)
    result += event({
      type: "task_complete",
      turn_id: id,
      last_agent_message: `## ${prefix}结论 ${index}\n\n合成结果，不代表正确性判断。\n\n<script>window.__unsafeFlow=true</script>\n\n![remote](https://invalid.example.test/tracker.png)\n\n[xss](javascript:alert(1))\n\n${"长回复用于检查节点原文阅读滚动位置。\n\n".repeat(55)}`,
    });
  return result;
}
const initial =
  record("session_meta", {
    id: "flow-qa-main",
    cwd: "/synthetic/alpha",
    source: "cli",
  }) + Array.from({ length: 105 }, (_, index) => turn(index)).join("");
const otherInitial =
  record("session_meta", {
    id: "flow-qa-other",
    cwd: "/synthetic/beta",
    source: "cli",
  }) + turn(0, "乙");
await fs.writeFile(rollout, initial);
await fs.writeFile(other, otherInitial);
const child = spawn(binary, ["--web", "--port", "0", "--no-open", "--all"], {
  env: {
    ...process.env,
    CODEX_HOME: codexHome,
    XDG_CONFIG_HOME: path.join(fixture, "config"),
  },
  stdio: ["ignore", "pipe", "pipe"],
});
let childError = "";
child.stderr.on("data", (chunk) => {
  childError += chunk;
});
const tabs = new Set();
async function cdp(endpoint, body) {
  const response = await fetch("http://localhost:3456" + endpoint, {
    ...(body === undefined ? {} : { method: "POST", body }),
    signal: AbortSignal.timeout(20000),
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
    `(async()=>{const deadline=Date.now()+15000;while(Date.now()<deadline){if(${condition})return true;await new Promise(r=>setTimeout(r,100))}return false})()`,
  );
  if (!passed)
    throw new Error(
      label +
        ": " +
        JSON.stringify(
          await evaluate(
            target,
            "({ready:document.readyState,text:document.body.innerText.slice(-1800)})",
          ),
        ),
    );
}
const input = (selector, value) =>
  `(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.value=${JSON.stringify(value)};e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`;
const key = (value, selector) =>
  `(async()=>{let e=document.querySelector(${JSON.stringify(selector)});if(e?.closest('#question-path')){const deadline=Date.now()+15000;while(document.querySelector('#question-path')?.getAttribute('aria-busy')==='true'&&Date.now()<deadline)await new Promise(r=>setTimeout(r,100));e=document.querySelector(${JSON.stringify(selector)})}e.focus({preventScroll:true});e.dispatchEvent(new KeyboardEvent('keydown',{key:${JSON.stringify(value)},bubbles:true,cancelable:true}));return true})()`;
const click = (target, selector) => cdp(`/click?target=${target}`, selector);
const selected = (index) =>
  `document.querySelector('#question-path [aria-current="true"]')?.dataset.flowTurn==='${index}'&&document.querySelector('#flow-inspector')?.textContent.includes('甲问题 ${index}：')`;
async function chooseMain(target) {
  await wait(
    target,
    "document.querySelectorAll('#sessions .session-row').length===2",
    "两主会话入口",
  );
  await evaluate(
    target,
    "(()=>{[...document.querySelectorAll('#sessions .session-row')].find(e=>e.textContent.includes('/synthetic/alpha')).click();return true})()",
  );
  await wait(
    target,
    "document.querySelector('#article')?.textContent.includes('甲问题 104：')||document.querySelector('#flow-inspector')?.textContent.includes('甲问题 104：')",
    "主会话最新问题",
  );
}
let appended = "";
try {
  const url = await new Promise((resolve, reject) => {
    let output = "";
    const timeout = setTimeout(
      () => reject(new Error("Web startup timeout: " + childError)),
      12000,
    );
    child.once("error", reject);
    child.once("exit", () => reject(new Error("Web exited: " + childError)));
    child.stdout.on("data", (chunk) => {
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
  const launcher = (await cdp("/new?url=about%3Ablank")).targetId;
  tabs.add(launcher);
  for (const width of [1280, 390]) {
    const qaUrl = url.replace("/#", `/?flowqa=${width}#`);
    await evaluate(
      launcher,
      `(()=>{document.body.replaceChildren();const b=document.createElement('button');b.id='launch';b.textContent='打开脉络验收';b.onclick=()=>window.open(${JSON.stringify(qaUrl)},'flow-qa-${width}','popup,width=${width},height=850');document.body.append(b);return true})()`,
    );
    await cdp(`/clickAt?target=${launcher}`, "#launch");
    let target;
    for (let attempt = 0; attempt < 40 && !target; attempt++) {
      target = (await cdp("/targets")).find((item) =>
        item.url.startsWith(qaUrl.split("#")[0]),
      )?.targetId;
      if (!target)
        await new Promise((resolve) => {
          setTimeout(resolve, 100);
        });
    }
    assert.ok(target, "独立窗口创建");
    tabs.add(target);
    await chooseMain(target);
    await click(target, "#view-flow");
    await wait(target, selected(104), "进入脉络保持所选轮");
    assert.ok(
      await evaluate(
        target,
        "document.querySelector('#article').hidden&&!document.querySelector('#flow-view').hidden",
      ),
      "视图切换",
    );
    await evaluate(target, key("g", "#question-path"));
    await wait(target, selected(0), "g 选择首轮");
    assert.equal(
      await evaluate(
        target,
        "document.querySelectorAll('.question-node').length",
      ),
      100,
    );
    await evaluate(target, key("ArrowRight", "#question-path"));
    await wait(target, selected(1), "方向键选择问题");
    await evaluate(target, key("j", "#question-path"));
    await wait(target, selected(2), "j 选择下一问题而非正文滚动");
    await evaluate(target, key("G", "#question-path"));
    await wait(target, selected(104), "G 跨页选择最新问题");
    assert.equal(
      await evaluate(
        target,
        "document.querySelectorAll('.question-node').length",
      ),
      5,
    );
    await click(target, "#flow-page-prev");
    await wait(
      target,
      "document.querySelectorAll('.question-node').length===100",
      "目录前页有界",
    );
    assert.ok(
      await evaluate(
        target,
        "document.querySelector('#flow-inspector').textContent.includes('甲问题 104：')",
      ),
      "翻页不改问题选择",
    );
    await click(target, "#flow-page-next");
    await wait(
      target,
      "document.querySelectorAll('.question-node').length===5",
      "目录后一页",
    );
    await evaluate(target, key("ArrowRight", '[data-flow-turn="100"]'));
    await wait(target, selected(101), "翻页后方向键以当前焦点问题为锚点");
    await evaluate(target, key("G", "#question-path"));
    await wait(target, selected(104), "恢复最新问题选择");

    await evaluate(target, input("#prompt-search", "搜索间隙"));
    await wait(
      target,
      "document.querySelectorAll('.question-node').length===2",
      "搜索只展示匹配问题",
    );
    assert.equal(
      await evaluate(
        target,
        "document.querySelector('.flow-edge').textContent",
      ),
      "中间 2 轮未展示",
      JSON.stringify(
        await evaluate(
          target,
          "({nodes:[...document.querySelectorAll('.question-node')].map(e=>({index:e.dataset.flowTurn,text:e.textContent.slice(0,80)})),edges:[...document.querySelectorAll('.flow-edge')].map(e=>e.textContent)})",
        ),
      ),
    );
    await click(target, '[data-flow-turn="1"]');
    await wait(target, selected(1), "搜索节点打开原文");
    assert.ok(
      await evaluate(
        target,
        "document.querySelector('#flow-inspector').textContent.length>1700",
      ),
      "原文非目录摘要",
    );
    await evaluate(target, input("#prompt-search", "不存在的合成问题"));
    await wait(
      target,
      "document.querySelectorAll('.question-node').length===0",
      "搜索空态",
    );
    assert.ok(
      await evaluate(
        target,
        "document.querySelector('#flow-inspector').textContent.includes('甲问题 1：')",
      ),
      "空态保留当前问题",
    );
    await click(target, "#flow-locate");
    await wait(target, selected(1), "定位当前恢复对应页");
    assert.equal(
      await evaluate(target, "document.querySelector('#prompt-search').value"),
      "",
    );

    await click(target, '[data-step="item-0"]');
    await wait(
      target,
      "document.querySelector('#flow-inspector').textContent.includes('甲进展 1')",
      "公开消息节点可读",
    );
    await click(target, "#flow-final-button");
    await wait(
      target,
      "document.querySelector('#flow-inspector').textContent.includes('甲结论 1')",
      "最终回复定位",
    );
    await evaluate(target, key("G", "#flow-inspector"));
    await wait(
      target,
      "(()=>{const e=document.querySelector('#flow-inspector');return e.scrollTop+e.clientHeight>=e.scrollHeight-3})()",
      "G 到节点原文底部",
    );
    await evaluate(target, key("g", "#flow-inspector"));
    await wait(
      target,
      "document.querySelector('#flow-inspector').scrollTop<2",
      "g 到节点原文开头",
    );
    const safety = await evaluate(
      target,
      "({unsafe:!!window.__unsafeFlow,images:document.querySelectorAll('#flow-view img').length,badLinks:[...document.querySelectorAll('#flow-view a')].some(a=>a.protocol==='javascript:'),external:performance.getEntriesByType('resource').filter(r=>/^https?:/.test(r.name)&&!r.name.startsWith(location.origin)).length,overflow:document.documentElement.scrollWidth>innerWidth+1})",
    );
    assert.deepEqual(safety, {
      unsafe: false,
      images: 0,
      badLinks: false,
      external: 0,
      overflow: false,
    });
    await evaluate(
      target,
      "(()=>{Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async(text)=>{window.__copiedFlow=text}}});return true})()",
    );
    await click(target, "#flow-copy");
    await wait(
      target,
      "window.__copiedFlow?.includes('甲结论 1')",
      "节点复制准确",
    );
    await click(target, "#flow-read");
    await wait(
      target,
      "!document.querySelector('#article').hidden&&document.querySelector('#flow-view').hidden",
      "返回阅读页",
    );
    assert.ok(
      await evaluate(
        target,
        "document.querySelector('#final-answer').textContent.includes('甲结论 1')",
      ),
      "阅读页对应同一问题",
    );
    await evaluate(target, key("v", "#reading-area"));
    await wait(
      target,
      "!document.querySelector('#flow-view').hidden",
      "v 切入脉络",
    );
    await evaluate(target, key("v", "#question-path"));
    assert.ok(
      await evaluate(
        target,
        "!document.querySelector('#article').hidden&&document.querySelector('#reading-area')===document.activeElement",
      ),
      "v 返回阅读页恢复可见阅读焦点",
    );
    await evaluate(target, key("g", ":focus"));
    assert.ok(
      await evaluate(
        target,
        "document.querySelector('#article').textContent.includes('甲问题 1：')",
      ),
      "返回阅读后 g 不切换当前问题",
    );
    await click(target, "#view-flow");
    await evaluate(target, key("g", "#question-path"));
    await wait(target, selected(0), "选择长活动轮");
    for (let attempt = 0; attempt < 12; attempt++) {
      if (await evaluate(target, "!!document.querySelector('#flow-before')"))
        break;
      const count = await evaluate(
        target,
        "document.querySelectorAll('.process-node').length",
      );
      assert.ok(count <= 65, "最多64活动加1问题节点");
      const first = await evaluate(
        target,
        "document.querySelectorAll('.process-node')[1]?.dataset.step",
      );
      await click(target, "#flow-after");
      await wait(
        target,
        `document.querySelectorAll('.process-node').length!==${count}||document.querySelectorAll('.process-node')[1]?.dataset.step!==${JSON.stringify(first)}`,
        "活动分页载入",
      );
    }
    assert.ok(
      await evaluate(
        target,
        "document.querySelectorAll('.process-node')[1]?.dataset.step==='item-64'",
      ),
      "活动窗口向后切换",
    );
    await click(target, "#flow-before");
    await wait(
      target,
      "document.querySelectorAll('.process-node')[1]?.dataset.step==='item-0'",
      "活动窗口可返回",
    );

    await click(target, "#flow-final-button");
    await wait(
      target,
      "document.querySelector('#flow-inspector').textContent.includes('甲结论 0')",
      "长轮最终回复",
    );
    if (width === 390) {
      await evaluate(
        target,
        "(()=>{const e=document.querySelector('#flow-inspector');e.scrollTop=180;window.__historyInspector=e;window.__historyScroll=e.scrollTop;window.__historyStep=document.querySelector('#flow-step-prompt');window.__readingScroll=document.querySelector('#reading-area').scrollTop;return true})()",
      );
      appended = turn(105, "甲", false);
      await fs.appendFile(rollout, appended);
      await wait(
        target,
        "document.querySelector('#latest').textContent.startsWith('1 条新问题')",
        "新轮次提示",
      );
      assert.ok(
        await evaluate(
          target,
          "document.querySelector('#flow-inspector').textContent.includes('甲结论 0')",
        ),
        "追加不抢走历史",
      );
      assert.ok(
        await evaluate(
          target,
          "window.__historyInspector.isConnected&&window.__historyStep.isConnected&&Math.abs(window.__historyInspector.scrollTop-window.__historyScroll)<2&&Math.abs(document.querySelector('#reading-area').scrollTop-window.__readingScroll)<2",
        ),
        "无关追加保留DOM与阅读位置",
      );
    }
    await evaluate(
      target,
      "(()=>{document.querySelector('#reading-area').scrollTop=0;document.querySelector('#flow-inspector').scrollTop=0;return true})()",
    );
    await cdp(
      `/screenshot?target=${target}&file=${encodeURIComponent(path.join(fixture, `flow-${width}.png`))}`,
    );
    if (width === 390) {
      await evaluate(target, key("G", "#question-path"));
      await wait(target, selected(105), "G 可到追加的新问题");
      await click(target, "#flow-final-button");
      await wait(
        target,
        "document.querySelector('#flow-inspector').textContent.includes('尚无明确标记的最终回复')",
        "未结束问题不猜测最终回复",
      );
    }
    await click(target, "#back");
    await wait(
      target,
      "!document.querySelector('#picker').hidden",
      "返回会话列表",
    );
    await evaluate(
      target,
      "(()=>{[...document.querySelectorAll('#sessions .session-row')].find(e=>e.textContent.includes('/synthetic/beta')).click();return true})()",
    );
    await wait(
      target,
      "document.querySelector('#flow-inspector')?.textContent.includes('乙问题 0：')",
      "会话切换不串原文",
    );
    await click(target, "#flow-final-button");
    await wait(
      target,
      "document.querySelector('#flow-inspector').textContent.includes('乙结论 0')",
      "会话切换最终回复正确",
    );
    assert.ok(
      await evaluate(
        target,
        "!document.querySelector('#flow-inspector').textContent.includes('甲结论')",
      ),
      "旧会话内容未泄漏到新节点",
    );
    console.log(
      `PASS 问题脉络 ${width}px：视图/键盘/搜索间隙/分页/节点/复制/会话隔离/安全/布局`,
    );
    await cdp(`/close?target=${target}`);
    tabs.delete(target);
  }
  const hash = (value) => createHash("sha256").update(value).digest("hex");
  assert.equal(
    hash(await fs.readFile(rollout)),
    hash(initial + appended),
    "仅脚本预期追加，应用不改写记录",
  );
  assert.equal(
    hash(await fs.readFile(other)),
    hash(otherInitial),
    "另一会话完全只读",
  );
  console.log("PASS 追加保护与合成记录只读校验");
} finally {
  for (const target of tabs)
    await cdp(`/close?target=${target}`).catch(() => {});
  if (child.exitCode === null) {
    child.kill("SIGINT");
    await Promise.race([
      once(child, "exit"),
      new Promise((resolve) => {
        setTimeout(resolve, 5000);
      }),
    ]);
    if (child.exitCode === null) child.kill("SIGTERM");
  }
  console.log(`合成验收目录与截图保留：${fixture}`);
}
