// Blocking, same-origin bootstrap: apply the saved theme before CSS or React paints.
// Keep the accepted values and key aligned with theme.ts; tests compare both paths.
(function () {
  var mode = "system";
  try {
    var saved = window.localStorage.getItem("questionTrail.theme");
    if (saved === "light" || saved === "dark") mode = saved;
  } catch {
    // Blocked storage must not prevent the page from opening.
  }
  var theme =
    mode === "system"
      ? window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light"
      : mode;
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
})();
