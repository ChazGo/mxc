const details = {
  invocation: {
    number: "01",
    owner: "Agent runtime",
    heading: "Tool invocation",
    summary:
      "The consumer starts with the tool it intends to run and request-local context such as the project root or explicit symbol overrides. OpenClaw performs a separate lookup for each tool.",
    code: 'tools: ["npm"]',
    items: [
      "OpenClaw passes a single-element tools array for each lookup",
      "Each returned floor is stored separately in OpenClaw's per-tool store",
      "Project context allows relative paths and tool settings to resolve correctly",
    ],
  },
  decision: {
    number: "2",
    owner: "OpenClaw owned",
    heading: "Stored policy for tool?",
    summary:
      "OpenClaw first checks its writable per-tool policy store. An existing record avoids a catalog lookup. A missing record checks whether automatic MXC floor retrieval is enabled before selecting the catalog or preset path.",
    code: "policyStore.get(toolId)",
    items: [
      "Store entries are keyed and persisted per tool",
      "A cache hit flows directly into policy composition",
      "A miss evaluates the OpenClaw plugin retrieval setting",
    ],
  },
  retrieval: {
    number: "3B",
    owner: "OpenClaw setting",
    heading: "Automatic floor retrieval?",
    summary:
      "The OpenClaw MXC plugin enables automatic floor retrieval by default. Users can opt out, in which case OpenClaw skips the MXC catalog request and uses the selected default MXC preset.",
    code: "automatic floor retrieval = enabled (default)",
    items: [
      "Enabled calls MXC once for the missing tool",
      "Disabled routes directly to the configured preset",
      "The setting controls lookup, not OS or enterprise ceilings",
    ],
  },
  preset: {
    number: "Opt-out",
    owner: "OpenClaw owned",
    heading: "Use default MXC preset",
    summary:
      "When automatic floor retrieval is disabled, OpenClaw applies the default tool policy selected in the MXC plugin directly to policy composition. It does not store that preset as the tool's floor.",
    code: "selected default preset",
    items: [
      "Current proposed presets are Locked Down, Recommended, and Unprotected",
      "Recommended is the proposed default preset",
      "The preset is evaluated per invocation and is not written to the per-tool store",
      "Re-enabling retrieval later still produces a fresh MXC catalog lookup",
    ],
  },
  stored: {
    number: "3A",
    owner: "OpenClaw owned",
    heading: "Load stored floor",
    summary:
      "When OpenClaw already has a stored record for the tool, it loads that tool's floor and proceeds without asking MXC to resolve the catalog again.",
    code: "storedFloor = policyStore.get(toolId)",
    items: [
      "The floor remains separate from user and learned overrides",
      "OpenClaw can retain catalog entry and version metadata with the record",
      "The loaded floor still passes through composition and hard ceilings",
    ],
  },
  api: {
    number: "MXC",
    owner: "MXC owned",
    heading: "Policy Store API",
    summary:
      "The MXC Sandbox Config Floors proposal places catalog lookup behind an MXC SDK API. MXC performs matching, dependency resolution, platform selection, and symbol expansion without modifying the consumer's policy.",
    code: "getSandboxConfigForTool(tools, ctx)",
    items: [
      "Returns an existing SandboxPolicy type",
      "The generic API can resolve multiple tools, but OpenClaw uses one tool per call",
      "Returns undefined when every requested tool is unknown",
    ],
  },
  floor: {
    number: "03",
    owner: "MXC output",
    heading: "Policy floor",
    summary:
      "The returned floor is a concrete SandboxPolicy describing the union of known minimum requirements for the matched tools. It is compatibility input, not authorization.",
    code: "SandboxPolicy | undefined",
    items: [
      "May include files, network endpoints, dependencies, and platform-resolved values",
      "A missing entry leaves the consumer's existing baseline in effect",
      "The consumer must decide whether the requested requirements are permitted",
    ],
  },
  catalog: {
    number: "MXC",
    owner: "MXC owned",
    heading: "Read reviewed floor",
    summary:
      "The SDK reads MXC's versioned, read-only catalog for the requested tool. The catalog describes minimum requirements and is not writable consumer state.",
    code: "catalog.lookup(tool)",
    items: [
      "Entries carry policy versions, dependencies, conditions, and provenance",
      "Consumers cannot write approvals or learned changes into the catalog",
      "An unknown tool produces no catalog result",
    ],
  },
  resolution: {
    number: "MXC",
    owner: "MXC owned",
    heading: "Resolve requirement",
    summary:
      "MXC follows dependencies, selects platform conditions, resolves symbols, and returns a literal SandboxPolicy floor or undefined.",
    code: "return SandboxPolicy | undefined",
    items: [
      "The resolver does not modify OpenClaw's policy",
      "The returned value covers one OpenClaw tool lookup",
      "Authorization remains an OpenClaw decision",
    ],
  },
  persist: {
    number: "3B",
    owner: "OpenClaw owned",
    heading: "Store floor by tool",
    summary:
      "OpenClaw stores a returned floor under the requested tool's record. If MXC returns undefined, OpenClaw keeps its application baseline rather than inventing an empty floor.",
    code: "policyStore.set(toolId, returnedFloor)",
    items: [
      "No combined multi-tool floor is stored",
      "Mutable user and learned changes remain consumer-owned layers",
      "The stored record then converges into the same composition path as a cache hit",
    ],
  },
  consumer: {
    number: "04",
    owner: "Agent owned",
    heading: "Policy composition",
    summary:
      "The agent combines the returned floor with its mutable policy state and product rules. This is where access profiles, user overrides, learned candidates, and approval decisions are applied.",
    code: "floor + consumer policy + approval",
    items: [
      "OpenClaw is the first consumer in the current proposal",
      "OpenClaw keeps returned floors separate by tool rather than storing a combined result",
      "Consumer settings and UI remain outside the read-only MXC catalog",
    ],
  },
  constraints: {
    number: "05",
    owner: "Hard ceilings",
    heading: "Effective constraints",
    summary:
      "OS, enterprise, and device restrictions remain authoritative. A catalog floor or user approval cannot widen the final request beyond these non-overridable ceilings.",
    code: "effective policy <= hard ceilings",
    items: [
      "Enterprise governance can require or prohibit controls",
      "Device and backend capabilities determine what can actually be enforced",
      "An unrealizable required policy should fail rather than silently run uncontained",
    ],
  },
  sandbox: {
    number: "06",
    owner: "Execution result",
    heading: "Final MXC sandbox",
    summary:
      "The consumer creates the final sandbox after evaluating tool requirements against its own authorization model and the non-overridable constraints of the OS, enterprise, and device.",
    code: "createConfigFromPolicy(effectivePolicy)",
    items: [
      "The existing MXC configuration path remains unchanged",
      "Contained execution receives only the effective composed policy",
      "The effective policy and its inputs should be auditable",
    ],
  },
};

const buttons = document.querySelectorAll("[data-detail]");
const number = document.querySelector("#detail-number");
const owner = document.querySelector("#detail-owner");
const heading = document.querySelector("#detail-heading");
const summary = document.querySelector("#detail-summary");
const code = document.querySelector("#detail-code code");
const list = document.querySelector("#detail-list");
const themeToggle = document.querySelector("#theme-toggle");

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

updateThemeToggle();

for (const button of buttons) {
  const showDetail = () => {
    const detail = details[button.dataset.detail];
    if (!detail) {
      return;
    }

    for (const item of buttons) {
      item.classList.toggle("active", item === button);
    }

    number.textContent = detail.number;
    owner.textContent = detail.owner;
    heading.textContent = detail.heading;
    summary.textContent = detail.summary;
    code.textContent = detail.code;
    list.replaceChildren(
      ...detail.items.map((item) => {
        const element = document.createElement("li");
        element.textContent = item;
        return element;
      }),
    );
  };

  button.addEventListener("click", showDetail);
  button.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      showDetail();
    }
  });
}
