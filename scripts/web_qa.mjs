// 合成数据端到端验收。浏览器操作仅经 web-access CDP Proxy，不访问已有标签页。
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { once } from "node:events";

const binary = path.resolve(process.argv[2] || "target/release/codex-nav");
const noWatch = process.argv.includes("--no-watch");
const proxy = "http://localhost:3456";
const fixture = await fs.mkdtemp(path.join(os.tmpdir(), "codex-nav-web-qa-"));
const home = path.join(fixture, "codex");
await fs.mkdir(path.join(home, "sessions"), { recursive: true });
const rollout = path.join(home, "sessions", "rollout-web-main.jsonl");
const record = (type, payload) => JSON.stringify({ type, payload }) + "\n";
const event = (payload) => record("event_msg", payload);
function turn(index, complete = true) {
  const id = `turn-${index}`;
  let text = event({ type: "task_started", turn_id: id });
  text += event({
    type: "user_message",
    message: `问题 ${index}：验证本地阅读 ${index === 0 ? "needle-first" : ""}\n${"这是合成的长输入，用来验证宽窄排版与首尾导航。\n".repeat(28)}`,
  });
  for (let i = 0; i < (index === 0 ? 75 : 12); i++) {
    text += record("response_item", {
      type: "function_call",
      name: "exec_command",
      call_id: `${id}-call-${i}`,
      arguments: JSON.stringify({ cmd: `echo synthetic-${i}` }),
    });
  }
  text += record("response_item", {
    type: "function_call_output",
    call_id: `${id}-call-0`,
    output: "Process exited with code 1\nsynthetic warning",
  });
  if (complete) {
    text += event({
      type: "task_complete",
      turn_id: id,
      last_agent_message: `## 最终结论 ${index}\n\n这是合成测试，不包含真实会话。\n\n- 保持只读\n- 保留历史位置\n\n\`\`\`js\nconst answer = 42;\n\`\`\`\n\n<script>window.__unsafeLog = true</script>\n\n![remote](https://invalid.example.test/tracker.png)\n\n[xss](javascript:alert(1))`,
    });
  }
  return text;
}
const initial =
  record("session_meta", {
    id: "web-qa-main",
    cwd: "/synthetic/project",
    source: "cli",
  }) + Array.from({ length: 105 }, (_, i) => turn(i)).join("");
await fs.writeFile(rollout, initial);
await fs.writeFile(
  path.join(home, "sessions", "rollout-web-child.jsonl"),
  record("session_meta", {
    id: "web-qa-child",
    source: { subagent: { thread_spawn: { parent_thread_id: "web-qa-main" } } },
  }) + turn(0),
);
const hash = (text) => createHash("sha256").update(text).digest("hex");
const child = spawn(
  binary,
  [
    "--web",
    "--port",
    "0",
    "--no-open",
    "--all",
    ...(noWatch ? ["--no-watch"] : []),
  ],
  {
    env: {
      ...process.env,
      CODEX_HOME: home,
      XDG_CONFIG_HOME: path.join(fixture, "config"),
    },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let childError = "";
child.stderr.on("data", (chunk) => {
  childError += chunk;
});
const tabs = new Set();
let launcher;
async function cdp(endpoint, body) {
  const response = await fetch(proxy + endpoint, {
    ...(body === undefined ? {} : { method: "POST", body }),
    signal: AbortSignal.timeout(15000),
  });
  const result = await response.json();
  if (!response.ok || result.error)
    throw new Error(`CDP ${endpoint.split("?")[0]}: ${JSON.stringify(result)}`);
  return result;
}
async function evaluate(target, code) {
  const result = await cdp(`/eval?target=${target}`, code);
  if (result.value?.qaError) throw new Error(result.value.qaError);
  return result.value;
}
async function wait(target, condition, label) {
  const result = await evaluate(
    target,
    `(async()=>{
    const deadline=Date.now()+12000;
    while(Date.now()<deadline){if(${condition})return true;await new Promise(r=>setTimeout(r,100));}
    return false;
  })()`,
  );
  if (!result) {
    const diagnostics = await evaluate(
      target,
      "({hidden:document.hidden,ready:document.readyState,text:document.body.innerText.slice(0,1600)})",
    );
    throw new Error(label + ": " + JSON.stringify(diagnostics));
  }
}
const input = (selector, value) =>
  `(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.value=${JSON.stringify(value)};e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`;
const key = (value, selector) =>
  `(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.focus();e.dispatchEvent(new KeyboardEvent('keydown',{key:${JSON.stringify(value)},bubbles:true,cancelable:true}));return true})()`;

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
  launcher = (await cdp("/new?url=about%3Ablank")).targetId;
  tabs.add(launcher);
  for (const width of [1280, 390]) {
    const qaUrl = url.replace("/#", `/?qa=${width}#`);
    // 用户手势打开独立测试窗口，真实 viewport 触发媒体查询，不改生产 CSP。
    await evaluate(
      launcher,
      `(()=>{document.body.replaceChildren();const b=document.createElement('button');b.id='launch';b.textContent='打开合成验收页面';b.onclick=()=>window.open(${JSON.stringify(qaUrl)},'codex-nav-qa-${width}','popup,width=${width},height=850');document.body.append(b);return true})()`,
    );
    await cdp(`/clickAt?target=${launcher}`, "#launch");
    let target;
    for (let attempt = 0; attempt < 30 && !target; attempt++) {
      const targets = await cdp("/targets");
      target = targets.find((item) =>
        item.url.startsWith(qaUrl.split("#")[0]),
      )?.targetId;
      if (!target)
        await new Promise((resolve) => {
          setTimeout(resolve, 100);
        });
    }
    assert.ok(target, "独立验收窗口已打开");
    tabs.add(target);
    await wait(
      target,
      "document.querySelectorAll('#sessions .session-row').length===1",
      "主会话列表加载且隐藏子代理",
    );
    const geometry = await evaluate(
      target,
      "({width:innerWidth,height:innerHeight,overflow:document.documentElement.scrollWidth>innerWidth+1,hash:location.hash})",
    );
    assert.equal(geometry.overflow, false, "会话列表无横向溢出");
    assert.equal(geometry.hash, "", "令牌已从地址栏移除");
    await evaluate(target, input("#session-search", "no-such-session-zz"));
    await wait(
      target,
      "document.querySelectorAll('#sessions .session-row').length===0",
      "会话搜索空态",
    );
    await evaluate(target, input("#session-search", ""));
    await wait(
      target,
      "document.querySelectorAll('#sessions .session-row').length===1",
      "会话搜索恢复",
    );
    await cdp(`/click?target=${target}`, "#sessions .session-row");
    await wait(
      target,
      "document.querySelector('#final-answer') && document.querySelector('#final-answer').textContent.includes('最终结论 104')",
      "初次打开定位最新轮最终回复",
    );
    await evaluate(target, input("#prompt-search", "needle-first"));
    await wait(
      target,
      "document.querySelectorAll('#turns .turn-btn').length===1",
      "真实 Prompt 搜索",
    );
    await cdp(`/click?target=${target}`, "#turns .turn-btn");
    await wait(
      target,
      "document.querySelector('#final-answer')?.textContent.includes('最终结论 0')",
      "搜索可打开历史轮",
    );
    await evaluate(target, input("#prompt-search", ""));
    await wait(
      target,
      "document.querySelectorAll('#turns .turn-btn').length>1",
      "目录分页恢复",
    );
    await evaluate(
      target,
      `(()=>{
      window.__qaFetch=window.fetch;window.__faults={detail:1,turns:1};window.__retried={detail:0,turns:0};
      window.fetch=async(...args)=>{
        const url=String(args[0]);const kind=url.includes('/turn/0?')?'detail':url.includes('/turns?')?'turns':null;
        if(kind&&window.__faults[kind]-->0)return new Response(JSON.stringify({error:'synthetic transient failure'}),{status:503,headers:{'Content-Type':'application/json'}});
        const result=await window.__qaFetch(...args);if(kind&&result.ok)window.__retried[kind]++;return result;
      };return true;
    })()`,
    );
    await cdp(`/click?target=${target}`, "#refresh");
    await wait(
      target,
      "window.__retried.detail>0&&window.__retried.turns>0&&document.querySelector('#error-banner').hidden",
      "相同revision的目录/正文失败会自动重试恢复",
    );
    await evaluate(target, "window.fetch=window.__qaFetch;true");
    await evaluate(target, key("f", "#reading-area"));
    assert.ok(
      await evaluate(
        target,
        "(()=>{const a=document.querySelector('#final-answer').getBoundingClientRect(),b=document.querySelector('#reading-area').getBoundingClientRect();return a.top<b.bottom&&a.bottom>b.top})()",
      ),
      "f 显示最终回复",
    );
    await evaluate(target, key("G", "#reading-area"));
    await wait(
      target,
      "(()=>{const e=document.querySelector('#reading-area');return e.scrollTop+e.clientHeight>=e.scrollHeight-3})()",
      "G 正文底部",
    );
    await evaluate(target, key("g", "#reading-area"));
    await wait(
      target,
      "document.querySelector('#reading-area').scrollTop<2",
      "g 正文顶部",
    );
    await evaluate(target, key("?", "#reading-area"));
    await wait(target, "document.querySelector('#help').open", "帮助可打开");
    await cdp(`/click?target=${target}`, "#close-help");
    assert.ok(
      await evaluate(target, "!document.querySelector('#help').open"),
      "帮助可关闭",
    );
    const safety = await evaluate(
      target,
      "({unsafe:!!window.__unsafeLog,images:document.querySelectorAll('#reading-area img').length,badLinks:[...document.querySelectorAll('#reading-area a')].some(a=>a.protocol==='javascript:'),external:performance.getEntriesByType('resource').filter(r=>/^https?:/.test(r.name)&&!r.name.startsWith(location.origin)).length,overflow:document.documentElement.scrollWidth>innerWidth+1})",
    );
    assert.deepEqual(safety, {
      unsafe: false,
      images: 0,
      badLinks: false,
      external: 0,
      overflow: false,
    });
    await cdp(`/click?target=${target}`, "#activity-summary");
    await cdp(`/click?target=${target}`, "#activity-more");
    await wait(
      target,
      "document.querySelectorAll('#activity-items .activity-item').length>8",
      "活动分页展开",
    );
    if (width === 1280) {
      for (let expected = 24; expected <= 64; expected += 8) {
        await cdp(`/click?target=${target}`, "#activity-more");
        await wait(
          target,
          `document.querySelectorAll('#activity-items .activity-item').length===${expected}`,
          "活动窗口逐页加载",
        );
      }
      await cdp(`/click?target=${target}`, "#activity-more");
      await wait(
        target,
        "document.querySelector('.activity-item')?.dataset.item==='64'&&document.querySelectorAll('.activity-item').length<=64",
        "下一组不无限累积 DOM",
      );
      await cdp(`/click?target=${target}`, "#activity-prev");
      await wait(
        target,
        "document.querySelector('.activity-item')?.dataset.item==='0'",
        "上一组活动可返回",
      );
      await cdp(`/click?target=${target}`, "#activity-more");
      await wait(
        target,
        "document.querySelectorAll('.activity-item').length===16",
        "活动窗口恢复",
      );
    }
    await evaluate(
      target,
      "Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async()=>{throw new Error('QA clipboard denied')}}});true",
    );
    await cdp(`/click?target=${target}`, "#copy-turn");
    await wait(
      target,
      "document.querySelector('#copy-fallback').open",
      "剪贴板不可用时提供手动复制",
    );
    assert.ok(
      await evaluate(
        target,
        "(()=>{const t=document.querySelector('#copy-text').value;return t.includes('USER')&&t.includes('FINAL ANSWER')&&t.includes('synthetic-74')})()",
      ),
      "复制包含未首屏加载的整轮保留内容",
    );
    await cdp(`/click?target=${target}`, "#close-copy");
    if (process.env.QA_SCREENSHOTS === "1") {
      await evaluate(target, key("f", "#reading-area"));
      await cdp(
        `/screenshot?target=${target}&file=${encodeURIComponent(path.join(fixture, `reader-${width}.png`))}`,
      );
    }
    if (width === 390) {
      await evaluate(
        target,
        "(()=>{window.__historyNode=document.querySelector('#final-answer');window.__itemCount=document.querySelectorAll('#activity-items .activity-item').length;window.__readPosition=document.querySelector('#reading-area').scrollTop;return true})()",
      );
      await fs.appendFile(rollout, turn(105, false));
      if (noWatch) {
        await new Promise((resolve) => {
          setTimeout(resolve, 1200);
        });
        assert.ok(
          await evaluate(
            target,
            "!/^\\d+ 条新问题/.test(document.querySelector('#latest').textContent)",
          ),
          "关闭监控不自动读取追加",
        );
        await cdp(`/click?target=${target}`, "#refresh");
      }
      await wait(
        target,
        "document.querySelector('#latest').textContent.startsWith('1 条新问题')",
        "独立新增提示",
      );
      assert.ok(
        await evaluate(
          target,
          "document.querySelector('#final-answer')?.textContent.includes('最终结论 0')",
        ),
        "新增轮次不抢走历史选择",
      );
      assert.ok(
        await evaluate(
          target,
          "window.__historyNode.isConnected&&document.querySelector('#activity').open&&document.querySelectorAll('#activity-items .activity-item').length===window.__itemCount&&Math.abs(document.querySelector('#reading-area').scrollTop-window.__readPosition)<2",
        ),
        "无关轮更新保持历史 DOM、展开分页和滚动位置",
      );
      await evaluate(target, key("G", "#turns"));
      await wait(
        target,
        "document.querySelector('.prompt-box')?.textContent.startsWith('问题 105：')",
        "G 目录最新轮",
      );
      await evaluate(target, key("f", "#reading-area"));
      assert.ok(
        await evaluate(
          target,
          "document.querySelector('#final-answer')?.textContent.includes('尚无明确标记的最终回复')",
        ),
        "无 final 不冒充最终答案",
      );
    }
    await cdp(`/click?target=${target}`, "#back");
    await wait(
      target,
      "!document.querySelector('#picker').hidden",
      "返回主会话选择",
    );
    console.log(
      `PASS Web 浏览器 ${geometry.width}×${geometry.height} (${noWatch ? "手动刷新" : "实时监控"})：真实数据 / 搜索 / f/g/G / 帮助 / XSS / 只读与无外部请求`,
    );
    await cdp(`/close?target=${target}`);
    tabs.delete(target);
  }
  const actual = await fs.readFile(rollout, "utf8");
  assert.equal(
    hash(actual),
    hash(initial + turn(105, false)),
    "仅验收脚本的预期追加，应用未改写源文件",
  );
  console.log("PASS 105 轮分页会话与实时历史保护，源文件内容一致");
} finally {
  for (const target of tabs) {
    await cdp(`/close?target=${target}`).catch(() => {});
  }
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
  console.log(`合成验收目录保留：${fixture}`);
}
