// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { describe, it } from 'node:test';
import assert from 'node:assert';
import * as path from 'node:path';
import {
  getSandboxConfigForTool,
  resolveConfigFloorCatalogForTest,
  validateConfigFloorCatalog,
} from '../../src/config-floors.js';

const projectRoot = path.resolve('test-project');
const npmCache = path.resolve('test-npm-cache');
const gitPrefix = path.resolve('test-git-prefix');
const nodePrefix = path.resolve('test-node-prefix');
const npmPrefix = path.resolve('test-npm-prefix');
const context = {
  projectRoot,
  symbols: {
    git_prefix: gitPrefix,
    node_prefix: nodePrefix,
    npm_prefix: npmPrefix,
    npm_cache: npmCache,
    npm_registry_host: 'registry.example.test',
  },
};

function catalog(entries: unknown[]): unknown {
  return { schemaVersion: '1', entries };
}

function entry(
  tool: string,
  policy: Record<string, unknown>,
  requires?: string[],
): Record<string, unknown> {
  return {
    tool,
    identity: [{ kind: 'invocation-name', names: [tool] }],
    ...(requires ? { requires } : {}),
    sandboxPolicy: { version: '0.8.0-alpha', ...policy },
  };
}

describe('getSandboxConfigForTool', () => {
  it('resolves a single repository catalog tool to a literal SandboxPolicy', () => {
    assert.deepStrictEqual(getSandboxConfigForTool(['git'], context), {
      version: '0.8.0-alpha',
      filesystem: {
        readonlyPaths: [gitPrefix],
        readwritePaths: [projectRoot],
      },
    });
  });

  it('unions multiple tools deterministically and lets read-write dominate read-only', () => {
    const first = getSandboxConfigForTool(['node', 'git'], context);
    const second = getSandboxConfigForTool(['git', 'node'], context);

    assert.deepStrictEqual(first, second);
    assert.deepStrictEqual(first, {
      version: '0.8.0-alpha',
      filesystem: {
        readonlyPaths: [gitPrefix, nodePrefix].sort(),
        readwritePaths: [projectRoot],
      },
    });
  });

  it('includes transitive dependencies', () => {
    const raw = catalog([
      entry('runtime', { filesystem: { readonlyPaths: ['C:\\runtime'] } }),
      entry('package-manager', { filesystem: { readwritePaths: ['${project_root}'] } }, ['runtime']),
      entry('wrapper', { network: { allowOutbound: true } }, ['package-manager']),
    ]);

    assert.deepStrictEqual(
      resolveConfigFloorCatalogForTest(['wrapper'], { projectRoot }, raw, 'windows'),
      {
        version: '0.8.0-alpha',
        filesystem: {
          readwritePaths: [projectRoot],
          readonlyPaths: ['C:\\runtime'],
        },
        network: {
          allowOutbound: true,
        },
      },
    );
  });

  it('terminates dependency cycles without duplicating requirements', () => {
    const raw = catalog([
      entry('a', { filesystem: { readonlyPaths: ['C:\\a'] } }, ['b']),
      entry('b', { filesystem: { readonlyPaths: ['C:\\b'] } }, ['a']),
    ]);

    assert.deepStrictEqual(
      resolveConfigFloorCatalogForTest(['a'], {}, raw, 'windows')?.filesystem?.readonlyPaths,
      ['C:\\a', 'C:\\b'],
    );
  });

  it('returns undefined when all requested tools are unknown', () => {
    assert.strictEqual(getSandboxConfigForTool(['unknown', 'also-unknown']), undefined);
  });

  it('returns known requirements when known and unknown tools are mixed', () => {
    assert.deepStrictEqual(
      getSandboxConfigForTool(['unknown', 'node'], context),
      {
        version: '0.8.0-alpha',
        filesystem: {
          readonlyPaths: [nodePrefix, projectRoot].sort(),
        },
      },
    );
  });

  it('resolves supported platform and caller symbols', () => {
    assert.deepStrictEqual(getSandboxConfigForTool(['npm'], context), {
      version: '0.8.0-alpha',
      filesystem: {
        readonlyPaths: [nodePrefix, npmPrefix].sort(),
        readwritePaths: [npmCache, projectRoot].sort(),
      },
      network: {
        allowOutbound: true,
        allowedHosts: ['registry.example.test'],
      },
    });
  });

  it('resolves project symbols consistently for each supported platform', () => {
    const raw = catalog([
      entry('tool', { filesystem: { readonlyPaths: ['${project_root}'] } }),
    ]);
    for (const platform of ['windows', 'linux', 'macos'] as const) {
      assert.deepStrictEqual(
        resolveConfigFloorCatalogForTest(['tool'], { projectRoot }, raw, platform),
        {
          version: '0.8.0-alpha',
          filesystem: { readonlyPaths: [projectRoot] },
        },
      );
    }
  });

  it('does not inspect or mutate a restrictive host policy', () => {
    const hostPolicy = {
      version: '0.8.0-alpha',
      network: { allowOutbound: false },
      filesystem: { deniedPaths: [projectRoot] },
    } as const;
    const before = structuredClone(hostPolicy);

    const floor = getSandboxConfigForTool(['npm'], context);

    assert.deepStrictEqual(hostPolicy, before);
    assert.strictEqual(floor?.network?.allowOutbound, true);
    assert.deepStrictEqual(hostPolicy.network, { allowOutbound: false });
  });
});

describe('config floor catalog validation', () => {
  it('rejects malformed fields instead of ignoring them', () => {
    const raw = catalog([
      entry('tool', { filesystem: { readonlyPaths: ['C:\\tool'] }, process: { command: 'bad' } }),
    ]);
    assert.throws(
      () => validateConfigFloorCatalog(raw),
      /unsupported field 'entries\[0\]\.sandboxPolicy\.process'/,
    );
  });

  it('rejects unresolved dependency references', () => {
    const raw = catalog([entry('tool', {}, ['missing'])]);
    assert.throws(
      () => validateConfigFloorCatalog(raw),
      /requires unknown tool 'missing'/,
    );
  });

  it('rejects unregistered embedded policy versions', () => {
    const raw = catalog([
      {
        ...entry('tool', {}),
        sandboxPolicy: { version: '99.0.0' },
      },
    ]);
    assert.throws(
      () => validateConfigFloorCatalog(raw),
      /is not a registered policy version/,
    );
  });

  it('rejects user-specific absolute paths', () => {
    const raw = catalog([
      entry('tool', { filesystem: { readonlyPaths: ['C:\\Users\\alice\\secret'] } }),
    ]);
    assert.throws(
      () => validateConfigFloorCatalog(raw),
      /contains a user-specific absolute path/,
    );
  });

  it('rejects wildcard grants', () => {
    const raw = catalog([
      entry('tool', { network: { allowOutbound: true, allowedHosts: ['*.example.com'] } }),
    ]);
    assert.throws(
      () => validateConfigFloorCatalog(raw),
      /contains a wildcard/,
    );
  });

  it('rejects malformed or unknown symbols', () => {
    assert.throws(
      () => validateConfigFloorCatalog(catalog([
        entry('tool', { filesystem: { readonlyPaths: ['${unknown_root}'] } }),
      ])),
      /contains unknown symbol 'unknown_root'/,
    );
    assert.throws(
      () => validateConfigFloorCatalog(catalog([
        entry('tool', { filesystem: { readonlyPaths: ['${BROKEN}'] } }),
      ])),
      /contains malformed symbol syntax/,
    );
  });
});
