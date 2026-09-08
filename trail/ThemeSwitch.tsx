import { useSyncExternalStore } from "react";
import { Monitor, Moon, Sun } from "lucide-react";
import { parseTheme, themeStore } from "./theme";

export function useTheme() {
  return useSyncExternalStore(themeStore.subscribe, themeStore.getSnapshot);
}

export default function ThemeSwitch() {
  const { mode } = useTheme();
  const Icon = mode === "system" ? Monitor : mode === "dark" ? Moon : Sun;
  return (
    <label className="theme-switch" title="切换主题，仅保存在当前浏览器">
      <Icon size={15} aria-hidden="true" />
      <select
        aria-label="主题"
        value={mode}
        onChange={(event) => themeStore.setMode(parseTheme(event.target.value))}
      >
        <option value="system">跟随系统</option>
        <option value="light">浅色</option>
        <option value="dark">深色</option>
      </select>
    </label>
  );
}
