// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/** Catalog platform selector values (design §4.4). */
export type CatalogPlatform = 'windows' | 'linux' | 'macos';

/** Catalog architecture selector values (design §4.4). */
export type CatalogArchitecture = 'x64' | 'arm64';

/**
 * The catalog-supported subset of MXC's `SandboxPolicy` authoring contract.
 *
 * This library deliberately does not import the MXC SDK. The shape is
 * structurally compatible with the MXC SDK `SandboxPolicy` type, so a consumer
 * can pass a reviewed, authorized result to `createConfigFromPolicy()`.
 * Backend-specific keys are not part of the catalog vocabulary.
 */
export interface CatalogSandboxPolicy {
  version: string;
  filesystem?: {
    deniedPaths?: string[];
    readonlyPaths?: string[];
    readwritePaths?: string[];
  };
  network?: {
    egress?: {
      default?: 'allow' | 'deny';
      allow?: CatalogNetworkRule[];
      deny?: CatalogNetworkRule[];
    };
    ingress?: {
      default?: 'allow' | 'deny';
      hostLoopback?: 'allow' | 'deny';
    };
  };
  ui?: {
    allowWindows?: boolean;
    clipboard?: 'none' | 'read' | 'write' | 'all';
    allowInputInjection?: boolean;
  };
  timeoutMs?: number;
}

export interface CatalogNetworkRule {
  to?: Array<{ cidr: string; except?: string[] }>;
  ports?: Array<{ protocol?: 'tcp' | 'udp' | 'icmp' | 'any'; port?: number; endPort?: number }>;
}

/** Runtime lookup input (design §5.1). */
export interface ToolCandidate {
  invocationName: string;
  packageUrl?: string;
  detectedVersion?: string;
}

/** A string is shorthand for `{ invocationName: tool }` (design §5.1). */
export type ToolInput = string | ToolCandidate;

/** Runtime lookup context, shared by every input in one lookup (design §5.1). */
export interface ResolveContext {
  projectRoot?: string;
  symbols?: Record<string, string>;
  platform?: CatalogPlatform;
  architecture?: CatalogArchitecture;
  catalogRevision?: string;
  allowWeakIdentityFallback?: boolean;
}

/** Strength of a matched identity predicate. */
export type IdentityStrength = 'strong' | 'weak';

/** Result of `resolveSandboxPolicyWithDiagnostics` (design §5.1). */
export interface SandboxConfigResolution {
  policy: CatalogSandboxPolicy | undefined;
  diagnostics: {
    catalogRevision: string;
    tools: Array<{
      inputIndex: number;
      matches: Array<{
        entryId: string;
        entryRevision: number;
        matchedIdentities: Array<{
          kind: string;
          strength: IdentityStrength;
        }>;
      }>;
    }>;
    resolvedDependencies: Array<{
      entryId: string;
      entryRevision: number;
      requiredVersionRange?: string;
    }>;
    warnings: string[];
  };
}

/** Inspection metadata (design §5.2). */
export type CatalogIdentityMetadata =
  | { kind: 'purl'; value: string; versionRange?: string }
  | { kind: 'invocation-name'; names: string[] };

/** Inspection metadata (design §5.2). It never exposes a policy body. */
export interface CatalogEntryMetadata {
  catalogRevision: string;
  entryId: string;
  entryRevision: number;
  displayName: string;
  identity: CatalogIdentityMetadata[];
  platformVariants: Array<{
    platform: CatalogPlatform;
    architecture?: CatalogArchitecture;
    dependencyEntryIds: string[];
    sandboxPolicyVersion: string;
  }>;
  provenance: {
    method: string;
    sourceRevision: string;
  };
}

/** Result of `getCatalogInfo()` (design §5.2). */
export interface CatalogInfo {
  catalogSchemaVersion: string;
  catalogRevision: string;
}
