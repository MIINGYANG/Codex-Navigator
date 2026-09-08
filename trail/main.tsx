import { createRoot } from "react-dom/client";
import App from "./App";
import "@xyflow/react/dist/style.css";
import "./app.css";
import { initTheme } from "./theme";

initTheme();
createRoot(document.getElementById("root")!).render(<App />);
