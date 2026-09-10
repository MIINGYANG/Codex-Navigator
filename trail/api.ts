const tokenKey = `questionTrail.token.${location.host}`;
const fragment = new URLSearchParams(location.hash.slice(1));
let token = fragment.get("token") || "";
try {
  if (token) sessionStorage.setItem(tokenKey, token);
  else token = sessionStorage.getItem(tokenKey) || "";
} catch {
  // Storage may be unavailable in private/restricted browsing. In-memory auth still works.
}
if (fragment.has("token")) {
  fragment.delete("token");
  history.replaceState(
    null,
    "",
    location.pathname + location.search + (fragment.size ? `#${fragment}` : ""),
  );
}

export async function api<T>(path: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(path, {
    headers: { "X-Codex-Nav-Token": token },
    signal,
    cache: "no-store",
  });
  if (!response.ok) {
    if (response.status === 401 || response.status === 403)
      throw new Error(
        "本地访问凭证已失效，请重新打开终端中带 token 的完整地址。",
      );
    throw new Error(
      `本地服务暂不可用（${response.status}），请确认 codex-nav --web 正在运行。`,
    );
  }
  return response.json() as Promise<T>;
}

export async function manageSession<T>(
  key: string,
  action: "rename" | "trash",
  body: { name: string } | { confirm: true },
): Promise<T> {
  const response = await fetch(
    `/api/session/${encodeURIComponent(key)}/${action}`,
    {
      method: "POST",
      headers: {
        "X-Codex-Nav-Token": token,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
      cache: "no-store",
    },
  );
  const result = await response.json().catch(() => null);
  if (!response.ok)
    throw new Error(
      result?.error || `会话操作失败（${response.status}），请稍后重试。`,
    );
  return result as T;
}

// Fetch-based SSE keeps the secret in a header, never the URL or server access log.
export async function subscribe(
  key: string,
  signal: AbortSignal,
  onChange: () => void,
  onConnection: (connected: boolean) => void,
) {
  let delay = 500;
  while (!signal.aborted) {
    try {
      const response = await fetch(
        `/api/trail/events?session=${encodeURIComponent(key)}`,
        {
          headers: { "X-Codex-Nav-Token": token },
          signal,
          cache: "no-store",
        },
      );
      if (!response.ok || !response.body)
        throw new Error("Event stream unavailable");
      onConnection(true);
      delay = 500;
      onChange(); // Reconcile revisions after a dropped connection, including file replacement.
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      try {
        while (!signal.aborted) {
          const { value, done } = await reader.read();
          if (done) break;
          buffer = (buffer + decoder.decode(value, { stream: true })).replace(
            /\r\n/g,
            "\n",
          );
          let boundary;
          while ((boundary = buffer.indexOf("\n\n")) !== -1) {
            const event = buffer.slice(0, boundary);
            buffer = buffer.slice(boundary + 2);
            if (/^data:/m.test(event)) onChange();
          }
          if (buffer.length > 65536) throw new Error("Invalid event frame");
        }
      } finally {
        reader.releaseLock();
      }
    } catch {
      /* Recover locally; a selected historical node and its viewport stay intact. */
    }
    if (signal.aborted) break;
    onConnection(false);
    await new Promise<void>((resolve) => {
      const done = () => {
        clearTimeout(timer);
        signal.removeEventListener("abort", done);
        resolve();
      };
      const timer = setTimeout(done, delay);
      signal.addEventListener("abort", done, { once: true });
    });
    delay = Math.min(5000, delay * 2);
  }
}
