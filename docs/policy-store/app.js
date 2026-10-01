const details = {
  invocation: {
    number: "01",
    owner: "Consumer runtime",
    heading: "Tool invocation",
    summary:
      "The consumer starts with the tool it intends to run and request-local context such as the project root or explicit symbol overrides. This illustrative per-tool flow looks up one tool at a time; the API also accepts an array for one combined policy.",
    code: 'resolveSandboxPolicy("npm", ctx)',
    items: [
      "A string input supplies only an invocation name, with no package or version evidence",
      "An object input can add packageUrl and detectedVersion; the caller verifies them",
      "Project context lets symbols such as ${project_root} resolve correctly",
    ],
  },
  decision: {
    number: "2",
    owner: "Consumer owned",
    heading: "Stored policy for tool?",
    summary:
      "In this illustrative per-tool flow, the consumer first checks its own writable policy store. An existing record avoids a catalog lookup. A missing record checks whether the consumer has explicitly enabled catalog lookup before choosing the catalog path or its restrictive baseline.",
    code: "policyStore.get(toolId)",
    items: [
      "Storage, keys, and persistence are consumer design choices",
      "A stored hit flows directly into policy composition",
      "A miss evaluates the consumer's own lookup setting",
    ],
  },
  retrieval: {
    number: "3B",
    owner: "Consumer setting",
    heading: "Catalog lookup enabled?",
    summary:
      "Existing callers never look up the catalog automatically. A consumer must explicitly enable or invoke lookup. Whether a given product, such as OpenClaw, offers that setting or turns it on is that product's decision; this page does not assume a default.",
    code: "catalog lookup = consumer opt-in",
    items: [
      "Enabled calls the catalog library once for the missing tool",
      "Disabled keeps the consumer's restrictive baseline",
      "Name-only matches also need allowWeakIdentityFallback: true",
      "The setting controls lookup, not OS or enterprise ceilings",
    ],
  },
  preset: {
    number: "Fallback",
    owner: "Consumer owned",
    heading: "Keep restrictive baseline",
    summary:
      "When lookup is disabled, or the library returns undefined because nothing matched or a required symbol could not be resolved, the consumer's restrictive baseline remains unchanged. Absence is never treated as an empty policy or a reason to run uncontained.",
    code: "policy === undefined → baseline unchanged",
    items: [
      "The baseline is the consumer's choice, not catalog data",
      "The baseline is not stored as the tool's floor",
      "A later invocation can look up a newly published floor",
    ],
  },
  stored: {
    number: "3A",
    owner: "Consumer owned",
    heading: "Load stored floor",
    summary:
      "When the consumer already holds an accepted record for the tool, it loads that floor and proceeds without resolving the catalog again. Installing a newer catalog does not rewrite previously accepted policies.",
    code: "storedFloor = policyStore.get(toolId)",
    items: [
      "The floor remains separate from user and learned overrides",
      "The record retains the catalog revision and contributing entry revisions",
      "The loaded floor still passes through composition and hard ceilings",
    ],
  },
  api: {
    number: "LIB",
    owner: "Catalog library",
    heading: "resolveSandboxPolicy",
    summary:
      "A standalone TypeScript, Rust, or .NET library resolves one tool or an array into one composed policy. It returns the corresponding MXC SDK's exact SandboxPolicy type, so a consumer can pass an accepted result to the SDK without conversion. These are in-process calls, not MXC SDK additions or a hosted service.",
    code: "resolveSandboxPolicy(tool, ctx)",
    items: [
      "resolveSandboxPolicyWithDiagnostics returns the same policy plus attribution and warnings",
      "Omitted context uses host platform, native architecture, and the installed catalog revision",
      "ctx.projectRoot and ctx.symbols override discovery and documented defaults",
      "Returns undefined when nothing matches; a library error stays distinct from absence",
    ],
  },
  floor: {
    number: "03",
    owner: "Library output",
    heading: "Policy floor",
    summary:
      "The returned floor is a candidate SandboxPolicy combining the known minimum requirements of every matched tool and dependency. It is compatibility input, not authorization or a guarantee of workflow success.",
    code: "SandboxPolicy | undefined",
    items: [
      "Composition preserves the least restrictive filesystem access the tools need",
      "A missing entry leaves the consumer's existing baseline in effect",
      "The consumer decides whether the requested access is permitted",
    ],
  },
  catalog: {
    number: "DATA",
    owner: "Catalog project",
    heading: "Read reviewed floor",
    summary:
      "The library reads a local, immutable catalog revision. Its content, including shared symbol defaults, is checked against the packaged digest on load. Lookup never downloads updates or contacts a service.",
    code: "ctx.catalogRevision ?? installed default",
    items: [
      "Entries carry identity, platform variants, dependencies, and provenance",
      "A requested revision that is unavailable is an error, not a substitution",
      "Consumers cannot write approvals or learned changes into the catalog",
    ],
  },
  resolution: {
    number: "LIB",
    owner: "Catalog library",
    heading: "Resolve requirement",
    summary:
      "The resolver adds every eligible matching entry and dependency, selects platform and architecture variants, resolves symbols, and composes filesystem floors. It never runs the candidate tool.",
    code: "return SandboxPolicy | undefined",
    items: [
      "Matching is additive: a stronger match does not suppress a weaker eligible one",
      "Invocation names match case-insensitively; paths follow the real filesystem's case rules",
      "Read-write supersedes overlapping read-only; a conflicting catalog deny is removed entirely",
      "Unresolved required symbols return no policy, never a partial one",
    ],
  },
  persist: {
    number: "3D",
    owner: "Consumer owned",
    heading: "Store accepted floor",
    summary:
      "If the consumer accepts a returned SandboxPolicy, it can store it under the tool's record with the catalog revision and contributing entry revisions from diagnostics. Undefined never enters this path.",
    code: "policyStore.set(toolId, acceptedFloor, revisions)",
    items: [
      "Catalog-derived requirements stay in a layer separate from user and learned policy",
      "The library never writes the consumer's store",
      "The stored record then follows the same composition path as a stored hit",
    ],
  },
  "floor-result": {
    number: "3C",
    owner: "Consumer owned",
    heading: "Floor returned?",
    summary:
      "The consumer branches on the library result. A SandboxPolicy can be reviewed and stored. Undefined means no policy was resolved, so the restrictive baseline stays in effect.",
    code: "result === undefined ? baseline : review(result)",
    items: [
      "SandboxPolicy flows to review and per-tool storage",
      "Undefined flows to the unchanged restrictive baseline",
      "Both branches converge at effective-policy composition",
    ],
  },
  consumer: {
    number: "04",
    owner: "Consumer owned",
    heading: "Policy composition",
    summary:
      "The consumer combines the floor with its own mutable policy state and product rules: access profiles, user overrides, learned candidates, and approval decisions. Its restrictive composition is separate from the catalog's least-restrictive floor composition.",
    code: "floor + consumer policy + approval",
    items: [
      "Diagnostics show superseded read-only paths and the full scope of removed catalog denies",
      "Caller-owned denies are never removed by catalog composition",
      "Consumer settings and UI remain outside the read-only catalog",
    ],
  },
  constraints: {
    number: "05",
    owner: "Hard ceilings",
    heading: "Effective constraints",
    summary:
      "OS, enterprise, device, and backend restrictions remain authoritative. A catalog floor or user approval cannot widen the final request beyond these ceilings.",
    code: "effective policy <= hard ceilings",
    items: [
      "Enterprise governance can require or prohibit controls",
      "Device and backend capabilities determine what can actually be enforced",
      "An unrealizable required policy fails closed rather than running uncontained",
    ],
  },
  sandbox: {
    number: "06",
    owner: "Execution result",
    heading: "Final MXC sandbox",
    summary:
      "The consumer passes its final, authorized policy to the existing MXC SDK. MXC neither references the catalog nor performs catalog lookup.",
    code: "createConfigFromPolicy(effectivePolicy)",
    items: [
      "The existing MXC configuration path remains unchanged",
      "Contained execution receives only the effective composed policy",
      "The consumer records revisions, matches, warnings, and approvals in its own audit trail",
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
