// 仅在隔离合成 CODEX_HOME 与自建 Chrome 窗口中验收会话管理。
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
const input = (target, selector, value) =>
  evaluate(
    target,
    `(()=>{const e=document.querySelector(${JSON.stringify(selector)});Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`,
  );
const cancelDialog = (target) =>
  evaluate(
    target,
    "(()=>{document.querySelector('dialog[open]').dispatchEvent(new Event('cancel',{cancelable:true}));return true})()",
  );
const view = (target) =>
  evaluate(
    target,
    "({selected:document.querySelector('.qt-card.is-selected')?.getAttribute('data-question-id'),transform:document.querySelector('.react-flow__viewport')?.style.transform})",
  );
const hasTitle = (name) =>
  `[...document.querySelectorAll('.session-item strong')].some(e=>e.textContent===${JSON.stringify(name)})`;

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
  const title = `ManagementQA-${process.pid}-${width}-${Date.now()}`;
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
  console.log(`管理验收：请求 ${width}px，实际 ${actual}px`);
  return actual;
}
async function launch(launcher, url, width) {
  const qaUrl = url.replace("/#", `/?managementqa=${width}-${Date.now()}#`);
  await evaluate(
    launcher,
    `(()=>{document.body.replaceChildren();const b=document.createElement('button');b.id='launch';b.textContent='打开管理验收';b.onclick=()=>window.open(${JSON.stringify(qaUrl)},'management-${width}-${Date.now()}','popup,width=${width},height=850');document.body.append(b);return true})()`,
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
async function openAction(target, name, action) {
  if (await evaluate(target, "innerWidth<1280"))
    await click(target, '[aria-label="打开会话列表"]');
  await evaluate(
    target,
    `(()=>{const row=[...document.querySelectorAll('.session-row')].find(e=>e.querySelector('.session-item strong')?.textContent===${JSON.stringify(name)});if(!row)throw new Error('会话不存在');row.querySelector('button[aria-label^="${action}会话："]').click();return true})()`,
  );
  await wait(
    target,
    "!!document.querySelector('.session-action-dialog[open]')",
    "管理对话框已打开",
  );
}
async function verifyTrash(file, env) {
  await assert.rejects(fs.stat(file.path), { code: "ENOENT" });
  const directory = path.join(env.XDG_DATA_HOME, "Trash", "files");
  const entries = await fs.readdir(directory);
  const candidates = entries.filter((name) =>
    name.startsWith(path.basename(file.path)),
  );
  assert.equal(candidates.length, 1, "只有本次合成文件的回收站副本");
  file.trashed = path.join(directory, candidates[0]);
  assert.equal(
    hash(await fs.readFile(file.trashed)),
    file.hash,
    "回收站内容保持完整",
  );
}
async function restore(file, env) {
  await assert.rejects(fs.stat(file.path), { code: "ENOENT" });
  try {
    await execute(
      "gio",
      [
        "trash",
        "--restore",
        `trash:///${encodeURIComponent(path.basename(file.trashed))}`,
      ],
      { env, timeout: 8000 },
    );
    await fs.stat(file.path);
  } catch {
    await assert.rejects(fs.stat(file.path), { code: "ENOENT" });
    await fs.rename(file.trashed, file.path);
    console.log(
      "当前 gio 不支持隔离回收站恢复，已精确移回合成文件；保留 trashinfo。",
    );
  }
  assert.equal(hash(await fs.readFile(file.path)), file.hash, "恢复后内容完整");
}

try {
  const launcher = (await cdp("/new?url=about%3Ablank")).targetId;
  tabs.add(launcher);
  for (const width of [1848, 390]) {
    const root = await fs.mkdtemp(
      path.join(os.tmpdir(), `codex-management-${width}-`),
    );
    roots.push(root);
    const home = path.join(root, "codex");
    const env = {
      ...process.env,
      CODEX_HOME: home,
      XDG_CONFIG_HOME: path.join(root, "config"),
      XDG_DATA_HOME: path.join(root, "data"),
    };
    const directory = path.join(home, "sessions", "2026", "09", "09");
    await fs.mkdir(directory, { recursive: true });
    await fs.mkdir(env.XDG_DATA_HOME, { recursive: true });
    const files = [];
    for (const [index, title] of [
      "管理合成主会话问题",
      "管理合成另一会话问题",
      "管理合成最后会话问题",
    ].entries()) {
      const id = randomUUID();
      const record = (type, payload) =>
        JSON.stringify({ type, payload, timestamp: "2026-09-09T09:00:00Z" }) +
        "\n";
      const cwd = path.join(root, `项目路径 ${index + 1}`);
      await fs.mkdir(cwd);
      const text =
        record("session_meta", {
          id,
          timestamp: "2026-09-09T09:00:00Z",
          cwd,
          originator: "codex_cli_rs",
          cli_version: "0.153.2",
          source: "cli",
          model_provider: "openai",
        }) +
        Array.from({ length: index === 0 ? 3 : 1 }, (_, turn) =>
          record("event_msg", {
            type: "user_message",
            message: turn === 0 ? title : `${title}：第 ${turn + 1} 步`,
            images: [],
            local_images: [],
          }),
        ).join("");
      const file = path.join(
        directory,
        `rollout-2026-09-09T09-00-00-${id}.jsonl`,
      );
      await fs.writeFile(file, text);
      await fs.utimes(
        file,
        new Date(),
        new Date(Date.now() + (3 - index) * 60000),
      );
      files.push({ path: file, title, cwd, hash: hash(text) });
    }
    let running = await start(home, env);
    const target = await launch(launcher, running.url, width);
    const [main, other, last] = files;
    await wait(
      target,
      "document.querySelectorAll('.session-item').length===3&&document.querySelectorAll('.qt-card').length===3",
      "合成会话已加载",
    );
    await click(target, '[data-question-id="q1"]');
    await wait(
      target,
      "!!document.querySelector('[data-question-id=\"q1\"].is-selected')",
      "历史问题已选中",
    );
    await pause(450);
    const before = await view(target);
    await click(
      target,
      '.session-heading-title button[aria-label^="重命名会话："]',
    );
    await input(target, ".session-name-field input", "   ");
    assert.equal(
      await evaluate(
        target,
        "document.querySelector('.session-action-dialog button[type=submit]').disabled",
      ),
      true,
      "空名称不可提交",
    );
    await input(target, ".session-name-field input", "取消的名称");
    await click(target, ".session-action-dialog footer button[type=button]");
    assert.equal(
      await evaluate(target, "document.querySelector('h1').textContent"),
      main.title,
      "取消改名保持原名称",
    );
    await click(
      target,
      '.session-heading-title button[aria-label^="重命名会话："]',
    );
    await cancelDialog(target);
    assert.deepEqual(
      await view(target),
      before,
      "Escape 取消不会改变选择与视口",
    );
    const name =
      "整理归档：中文项目路径与历史问题".repeat(2) +
      '<img src=x onerror="window.__managementXSS=1">';
    await click(
      target,
      '.session-heading-title button[aria-label^="重命名会话："]',
    );
    await input(target, ".session-name-field input", `  ${name}  `);
    await cdp(
      `/screenshot?target=${target}&file=${encodeURIComponent(path.join(root, "rename-dialog.png"))}`,
    );
    await click(target, ".session-action-dialog button[type=submit]");
    await wait(
      target,
      `!document.querySelector('.session-action-dialog')&&document.querySelector('h1')?.textContent===${JSON.stringify(name)}&&${hasTitle(name)}`,
      "名称保存并同步列表与当前会话",
    );
    assert.deepEqual(await view(target), before, "改名保留历史选择及画布视口");
    assert.equal(
      await evaluate(
        target,
        "!!window.__managementXSS||!!document.querySelector('h1 img,.session-item strong img')",
      ),
      false,
      "名称中的 HTML 仅作为文字显示",
    );
    assert.equal(
      hash(await fs.readFile(main.path)),
      main.hash,
      "重命名不修改会话正文",
    );
    await click(target, '[aria-label="搜索所有问题"]');
    await input(target, '[aria-label="搜索问题原文"]', main.title);
    await wait(
      target,
      `[...document.querySelectorAll('.result-group h3')].some(e=>e.textContent===${JSON.stringify(name)})`,
      "搜索分组显示正式名称",
    );
    await cancelDialog(target);
    const currentUrl = await evaluate(target, "location.href");
    await cdp(
      `/navigate?target=${target}&url=${encodeURIComponent(currentUrl)}`,
    );
    await wait(
      target,
      `document.querySelector('h1')?.textContent===${JSON.stringify(name)}&&${hasTitle(name)}`,
      "刷新后正式名称仍保留",
    );
    await cdp(
      `/screenshot?target=${target}&file=${encodeURIComponent(path.join(root, "renamed-success.png"))}`,
    );

    await openAction(target, other.title, "删除");
    assert.ok(
      await evaluate(
        target,
        `document.querySelector('.session-action-dialog').textContent.includes(${JSON.stringify(other.cwd)})`,
      ),
      "删除确认显示对应项目路径",
    );
    await click(target, ".session-action-dialog footer button[type=button]");
    assert.equal(
      hash(await fs.readFile(other.path)),
      other.hash,
      "取消删除不写入文件",
    );
    assert.equal(
      await evaluate(
        target,
        "document.querySelectorAll('.session-item').length",
      ),
      3,
      "取消删除保留列表",
    );
    await openAction(target, other.title, "删除");
    await cdp(
      `/screenshot?target=${target}&file=${encodeURIComponent(path.join(root, "trash-dialog.png"))}`,
    );
    await click(target, ".session-action-dialog button[type=submit]");
    await wait(
      target,
      `!document.querySelector('.session-action-dialog')&&document.querySelectorAll('.session-item').length===2&&!(${hasTitle(other.title)})`,
      "非当前会话已删除",
    );
    assert.equal(
      await evaluate(target, "document.querySelector('h1').textContent"),
      name,
      "删除其他会话保持当前会话",
    );
    await verifyTrash(other, env);
    await openAction(target, name, "删除");
    await click(target, ".session-action-dialog button[type=submit]");
    await wait(
      target,
      `!document.querySelector('.session-action-dialog')&&document.querySelectorAll('.session-item').length===1&&document.querySelector('h1')?.textContent===${JSON.stringify(last.title)}`,
      "删除当前会话后选择剩余会话",
    );
    await verifyTrash(main, env);
    await openAction(target, last.title, "删除");
    await click(target, ".session-action-dialog button[type=submit]");
    await wait(
      target,
      "!document.querySelector('.session-action-dialog')&&document.querySelectorAll('.session-item').length===0&&document.querySelectorAll('.qt-card').length===0",
      "删除最后会话后显示空态",
    );
    await verifyTrash(last, env);
    await cdp(
      `/screenshot?target=${target}&file=${encodeURIComponent(path.join(root, "trash-success.png"))}`,
    );
    await restore(main, env);
    await stop(running.service);
    running = await start(home, env);
    await cdp(
      `/navigate?target=${target}&url=${encodeURIComponent(running.url)}`,
    );
    await wait(
      target,
      `document.querySelectorAll('.session-item').length===1&&document.querySelector('h1')?.textContent===${JSON.stringify(name)}&&document.querySelectorAll('.qt-card').length===3`,
      "恢复后重启服务显示会话及正式名称",
    );
    assert.equal(
      hash(await fs.readFile(main.path)),
      main.hash,
      "恢复全程没有改写正文",
    );
    await cdp(
      `/screenshot?target=${target}&file=${encodeURIComponent(path.join(root, "restored-success.png"))}`,
    );
    await cdp(`/close?target=${target}`);
    tabs.delete(target);
    await stop(running.service);
    console.log(
      `PASS ${width}px：改名持久化、搜索、XSS 纯文本、视口保护、三类删除与恢复。`,
    );
  }
} finally {
  for (const target of tabs)
    await cdp(`/close?target=${target}`).catch(() => {});
  for (const service of services) await stop(service);
  for (const root of roots) console.log(`合成数据、回收站与截图保留：${root}`);
}
