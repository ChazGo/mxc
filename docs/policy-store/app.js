const details = {
  invocation: {
    number: "01",
    owner: "Consumer runtime",
    heading: "Tool invocation",
    summary:
      "The consumer starts with one tool or an array and request-local context such as the project root or explicit symbol overrides. Each candidate can include strong package identity, a fallback invocation name, a detected version, and an operation intent.",
    code: 'resolveSandboxPolicy({ invocationName: "git", intent: "push" }, ctx)',
    items: [
      "packageUrl is the strong match; invocationName is the explicitly enabled fallback",
      "Invocation-name matching ignores case on Windows and macOS and is exact on Linux",
      "An object input can add intent and detectedVersion; the caller verifies all supplied identity",
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
      "Enabled calls the MXC SDK once for the missing tool",
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
      "When lookup is disabled, or the SDK returns undefined because nothing matched or a required symbol could not be resolved, the consumer's restrictive baseline remains unchanged. Absence is never treated as an empty policy or a reason to run uncontained.",
    code: "policy === undefined → baseline unchanged",
    items: [
      "The baseline is the consumer's choice, not catalog data",
      "The baseline is not stored as the tool's floor",
      "A later SDK release can include a newly reviewed floor",
    ],
  },
  stored: {
    number: "3A",
    owner: "Consumer owned",
    heading: "Load stored floor",
    summary:
      "When the consumer already holds an accepted record for the tool, it loads that floor and proceeds without resolving the catalog again. Updating to an SDK release with a newer bundled catalog does not rewrite previously accepted policies.",
    code: "storedFloor = policyStore.get(toolId)",
    items: [
      "The floor remains separate from user and learned overrides",
      "The record retains the catalog revision and contributing entry revisions",
      "The loaded floor still passes through composition and hard ceilings",
    ],
  },
  api: {
    number: "LIB",
    owner: "MXC SDK",
    heading: "resolveSandboxPolicy",
    summary:
      "Proposed TypeScript, Rust, and .NET MXC SDK APIs resolve one tool or an array into one composed SandboxPolicy. The Rust mxc_policy_store core serves every SDK; Node and .NET wrap it through mxc_ffi. The APIs are pending sign-off, target a later release, and are not part of MXC 1.0.",
    code: "resolveSandboxPolicy(tool, ctx)",
    items: [
      "resolveSandboxPolicyWithDiagnostics returns the same policy plus attribution and warnings",
      "The proposed names may change during API review, including removal of Sandbox",
      "Omitted context uses host platform, native architecture, and the installed catalog revision",
      "ctx.projectRoot and ctx.symbols override discovery and documented defaults",
      "Returns undefined when no pair contributes; .NET failures throw MxcException with Reason",
    ],
  },
  floor: {
    number: "03",
    owner: "SDK output",
    heading: "Policy floor",
    summary:
      "The returned floor is a candidate SandboxPolicy combining the known minimum requirements of every contributing tool-plus-intent pair and dependency. It is compatibility input, not authorization or a guarantee of workflow success.",
    code: "SandboxPolicy | undefined",
    items: [
      "Filesystem and scoped network requirements combine to satisfy every contributing pair",
      "Unmatched tools and unsupported intents contribute nothing while other inputs still resolve",
      "The consumer decides whether the requested access is permitted",
    ],
  },
  catalog: {
    number: "DATA",
    owner: "MXC SDK",
    heading: "Read reviewed floor",
    summary:
      "The SDK reads a local, immutable catalog revision bundled statically with that SDK release. Its content, including shared symbol defaults, is checked against the packaged digest on load. Lookup never downloads updates or contacts a service.",
    code: "ctx.catalogRevision ?? installed default",
    items: [
      "Overlays use policyAdditions, intentAdditions for default intents, and newIntents for new names",
      "Every entry declares one of five version schemes: npm, semver, pypi, nuget, or intdot",
      "A requested revision that is unavailable is an error, not a substitution",
      "Consumers cannot write approvals or learned changes into the catalog",
    ],
  },
  resolution: {
    number: "LIB",
    owner: "MXC SDK",
    heading: "Resolve requirement",
    summary:
      "For each input, the resolver selects one identity match, starts with its conservative default, applies at most one add-only platform/architecture overlay and one non-overlapping version overlay, then selects intent. It never runs the candidate tool.",
    code: "return SandboxPolicy | undefined",
    items: [
      "Valid uncovered versions use the default with version_out_of_range; unparseable versions contribute nothing",
      "An unsupported named intent contributes nothing instead of falling back to base or every intent",
      "Dependencies contribute default and platform base only unless their reference names intents",
      "Overlapping catalog filesystem and egress denies are removed in full and diagnosed",
      "Unsupported compositions fail rather than approximating broader access",
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
      "The SDK never writes the consumer's store",
      "The stored record then follows the same composition path as a stored hit",
    ],
  },
  "floor-result": {
    number: "3C",
    owner: "Consumer owned",
    heading: "Floor returned?",
    summary:
      "The consumer branches on the SDK result. A SandboxPolicy can be reviewed and stored. Undefined means no policy was resolved, so the restrictive baseline stays in effect.",
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
      "A no-network pair does not veto scoped network access required by another pair",
      "An overlapping catalog egress deny is removed, never treated as a composition conflict",
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
      "The consumer passes its final, authorized policy through the MXC SDK after the proposed lookup API resolves the SDK's bundled catalog.",
    code: "createConfigFromPolicy(effectivePolicy)",
    items: [
      "The existing MXC configuration path remains unchanged",
      "The catalog schema stays compatible within MXC 1.x",
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
