const themeToggle = document.querySelector("#theme-toggle");
const tabs = [...document.querySelectorAll(".scope-tab")];
const panels = [...document.querySelectorAll(".scope-panel")];

function updateThemeToggle() {
  const dark = document.documentElement.dataset.theme === "dark";
  themeToggle.textContent = dark ? "Light theme" : "Dark theme";
  themeToggle.setAttribute("aria-pressed", String(dark));
}

themeToggle.addEventListener("click", () => {
  const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
  document.documentElement.dataset.theme = next;
  localStorage.setItem("mxc-policy-store-theme", next);
  updateThemeToggle();
});

for (const tab of tabs) {
  tab.addEventListener("click", () => {
    for (const candidate of tabs) {
      const selected = candidate === tab;
      candidate.classList.toggle("active", selected);
      candidate.setAttribute("aria-selected", String(selected));
    }

    for (const panel of panels) {
      panel.hidden = panel.id !== tab.dataset.tab;
    }
  });
}

updateThemeToggle();
