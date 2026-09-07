import { questionPath, processPath, statusLabel } from "./state.mjs";

function node(tag, className, text) {
  const result = document.createElement(tag);
  if (className) result.className = className;
  if (text !== undefined) result.textContent = text;
  return result;
}

function action(text, id, callback) {
  const result = node("button", "", text);
  result.type = "button";
  result.id = id;
  result.addEventListener("click", callback);
  return result;
}

// A bounded projection of recorded evidence, never an inferred reasoning graph.
export function renderFlow(root, data, actions) {
  const workspaceKey = JSON.stringify([
    data.detail?.generation,
    data.detail?.turn.index,
    data.detail?.turn.revision,
    data.inspected,
    data.start,
    data.next,
    data.items.map((item) => item.index),
    data.activityLoading,
  ]);
  const stableWorkspace = root.dataset.workspaceKey === workspaceKey;
  const scroll = root.querySelector("#question-path")?.scrollLeft || 0;
  const stepScroll = root.querySelector("#process-path")?.scrollTop || 0;
  const focusId = root.contains(document.activeElement)
    ? document.activeElement.id
    : null;
  const fragment = document.createDocumentFragment();
  const intro = node("div", "flow-intro");
  intro.append(
    node("div", "eyebrow", "FOLLOW YOUR QUESTIONS"),
    node("h1", "", "一个问题，如何走到这里。"),
    node(
      "p",
      "",
      "沿着问题回看，点开每一步记录。连线表示记录顺序，不推断因果，也不还原未公开的内部思维。",
    ),
  );
  intro.append(
    node("div", "flow-legend", "● 当前问题　— 记录顺序　⋯ 中间有未展示记录"),
  );
  fragment.append(intro);
  const toolbar = node("div", "flow-path-toolbar");
  const pages = node("div", "flow-pagination");
  const prev = action("← 前一页", "flow-page-prev", () => actions.page(-1));
  const next = action("后一页 →", "flow-page-next", () => actions.page(1));
  prev.disabled = data.offset === 0 || data.loading;
  next.disabled = data.offset + data.pageSize >= data.total || data.loading;
  pages.append(
    prev,
    node(
      "span",
      "",
      data.total
        ? `${data.offset + 1}–${Math.min(data.offset + data.pageSize, data.total)} / ${data.total}${data.query ? " 匹配" : " 轮"}`
        : "暂无匹配问题",
    ),
    next,
  );
  const locate = action("定位当前问题", "flow-locate", actions.locate);
  locate.disabled = data.selected === null;
  toolbar.append(pages, locate);
  fragment.append(toolbar);
  const path = node("nav", "question-path");
  path.id = "question-path";
  path.tabIndex = 0;
  path.setAttribute("aria-label", "问题记录顺序；方向键选择问题，Enter 查看");
  path.setAttribute("aria-busy", String(data.loading));
  const projection = questionPath(data.turns, data.selected);
  projection.nodes.forEach((turn, i) => {
    if (i) {
      const edge = projection.edges[i - 1];
      const link = node(
        "span",
        `flow-edge${edge.skipped ? " is-gap" : ""}`,
        edge.label,
      );
      path.append(link);
    }
    const item = action("", `flow-question-${turn.index}`, () =>
      actions.select(turn.index),
    );
    item.className = "question-node";
    item.dataset.flowTurn = turn.index;
    item.setAttribute("aria-current", String(turn.index === data.selected));
    item.tabIndex =
      turn.index === data.selected ||
      (!projection.nodes.some((t) => t.index === data.selected) && i === 0)
        ? 0
        : -1;
    item.append(
      node(
        "span",
        "flow-number",
        `问题 ${String(turn.ordinal).padStart(2, "0")}`,
      ),
      node(
        "span",
        "flow-question",
        turn.preview || "（图片或未记录文字的问题）",
      ),
      node(
        "span",
        "flow-status",
        `${statusLabel(turn.status)}${turn.errors ? ` · !${turn.errors} 活动告警` : ""}${turn.has_final ? " · 有最终回复" : ""}`,
      ),
    );
    path.append(item);
  });
  if (!projection.nodes.length)
    path.append(
      node(
        "p",
        "empty",
        data.loading
          ? "正在读取问题路径…"
          : "没有匹配的问题。可清空侧栏搜索，或选择其他会话。",
      ),
    );
  path.addEventListener("keydown", (event) => {
    const direction = {
      ArrowRight: "next",
      ArrowDown: "next",
      ArrowLeft: "previous",
      ArrowUp: "previous",
      Home: "start",
      End: "end",
    }[event.key];
    if (direction && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      actions.move(direction);
    }
  });
  fragment.append(path);
  if (
    data.selected !== null &&
    !projection.nodes.some((t) => t.index === data.selected)
  )
    fragment.append(
      node(
        "p",
        "omission-note",
        "当前问题不在这一页或搜索结果中；下方仍保留当前问题。点击“定位当前问题”清空筛选并回到对应页。",
      ),
    );

  if (!data.detail) {
    fragment.append(
      node(
        "p",
        "empty",
        data.error
          ? "当前问题读取失败。按 r 或点击顶部刷新重试。"
          : data.selected === null
            ? "选择一个问题，查看已有的执行记录。"
            : "正在读取这个问题的过程…",
      ),
    );
  } else {
    const workspace = node("div", "flow-workspace");
    const process = node("section", "flow-process");
    const head = node("div", "flow-process-head");
    head.append(
      node("h2", "", `问题 ${data.detail.turn.ordinal} · 已记录的过程`),
    );
    head.append(
      action("最终回复 ↗", "flow-final-button", () => actions.inspect("final")),
    );
    process.append(
      head,
      node(
        "p",
        "flow-process-note",
        `${data.detail.items_total} 条活动记录 · 点击节点查看原文${data.activityLoading ? " · 读取中" : ""}`,
      ),
    );
    const list = node("ol", "process-path");
    list.id = "process-path";
    list.setAttribute("aria-label", "当前问题的记录步骤");
    const steps = processPath(data.detail, data.items, data.start, data.next);
    const chosen =
      data.inspected === "final"
        ? null
        : steps.find(
            (step) => step.id === data.inspected && step.kind !== "gap",
          ) || steps[0];
    for (const step of steps) {
      const row = node("li", step.kind === "gap" ? "flow-gap" : "");
      if (step.kind === "gap") {
        const control = action(step.label, `flow-${step.id}`, () =>
          actions.more(
            step.id === "before" ? Math.max(0, data.start - 64) : data.next,
          ),
        );
        control.disabled = data.activityLoading;
        row.append(control);
      } else {
        const control = action("", `flow-step-${step.id}`, () =>
          actions.inspect(step.id),
        );
        control.className = "process-node";
        control.dataset.step = step.id;
        control.setAttribute("aria-current", String(chosen?.id === step.id));
        control.append(
          node(
            "span",
            "process-order",
            step.id === "prompt"
              ? "Q"
              : String(step.itemIndex + 1).padStart(2, "0"),
          ),
          node("span", "process-label", step.label),
          node("span", "process-preview", step.text || "（无文字内容）"),
        );
        if (step.isError)
          control.append(
            node("span", "warning", "活动告警 · 不代表最终结果错误"),
          );
        row.append(control);
      }
      list.append(row);
    }
    list.addEventListener("keydown", (event) => {
      if (event.ctrlKey || event.metaKey || event.altKey) return;
      const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
      if (!keys.includes(event.key)) return;
      const controls = [...list.querySelectorAll("button:not(:disabled)")];
      const at = controls.indexOf(document.activeElement);
      const target =
        event.key === "Home"
          ? 0
          : event.key === "End"
            ? controls.length - 1
            : Math.max(
                0,
                Math.min(
                  controls.length - 1,
                  at + (event.key === "ArrowDown" ? 1 : -1),
                ),
              );
      event.preventDefault();
      controls[target]?.focus();
    });
    process.append(list);
    const inspector = node("section", "flow-inspector");
    inspector.id = "flow-inspector";
    inspector.tabIndex = -1;
    inspector.setAttribute("aria-label", "选中节点原文");
    if (data.inspected === "final") {
      inspector.append(
        node("p", "section-label", "FINAL ANSWER"),
        node("h2", "", "最终回复"),
      );
      inspector.append(
        data.detail.final_answer
          ? actions.markdown(data.detail.final_answer.text)
          : node(
              "p",
              "empty",
              "尚无明确标记的最终回复。请查看已记录的步骤，不会把普通消息猜成最终答案。",
            ),
      );
    } else if (chosen) {
      inspector.append(
        node(
          "p",
          "section-label",
          chosen.id === "prompt"
            ? "YOUR PROMPT"
            : `RECORD ${chosen.itemIndex + 1}`,
        ),
        node("h2", "", chosen.label),
      );
      inspector.append(
        chosen.kind === "agent_message"
          ? actions.markdown(chosen.text || "")
          : node(
              "div",
              "flow-fulltext",
              chosen.text || "（仅有图片或未记录文字）",
            ),
      );
      if (chosen.id === "prompt") {
        const prompt = data.detail.turn.prompt;
        if (prompt.images_count)
          inspector.append(
            node(
              "p",
              "omission-note",
              `包含 ${prompt.images_count} 张图片，不加载图片附件。`,
            ),
          );
        if (prompt.omitted_bytes)
          inspector.append(
            node(
              "p",
              "omission-note",
              `有 ${prompt.omitted_bytes} 字节问题原文因预算省略；此处仅展示已保留内容。`,
            ),
          );
      }
    }
    const detailActions = node("div", "flow-detail-actions");
    const text =
      data.inspected === "final"
        ? data.detail.final_answer?.text
        : chosen?.text;
    const copy = action("复制节点", "flow-copy", () =>
      actions.copy(text || "", "节点内容"),
    );
    copy.disabled = !text;
    detailActions.append(
      copy,
      action("在阅读页打开此问题 ↗", "flow-read", actions.read),
    );
    inspector.append(
      detailActions,
      node(
        "p",
        "result-note",
        "这里展示已记录的内容与先后顺序，不判断答案正确性。",
      ),
    );
    workspace.append(process, inspector);
    fragment.append(workspace);
  }
  // Keep the selected record DOM (including selection and inner scroll) when
  // unrelated questions update. Reconcile only changed top-level sections.
  const children = [...fragment.children];
  for (const [index, child] of children.entries()) {
    const previous = root.children[index];
    if (
      stableWorkspace &&
      child.classList.contains("flow-workspace") &&
      previous?.classList.contains("flow-workspace")
    )
      continue;
    if (previous) root.replaceChild(child, previous);
    else root.append(child);
  }
  while (root.children.length > children.length) root.lastElementChild.remove();
  root.dataset.workspaceKey = workspaceKey;
  root.querySelector("#question-path").scrollLeft = scroll;
  const steps = root.querySelector("#process-path");
  if (steps) steps.scrollTop = stepScroll;
  if (focusId) document.getElementById(focusId)?.focus({ preventScroll: true });
}
