// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/** Parsed package URL, reduced to the parts identity matching uses. */
export interface ParsedPurl {
  /** `type/[namespace/]name` with the type lower-cased. Version-free. */
  key: string;
  version?: string;
}

/**
 * Parses a package URL (`pkg:type/namespace/name@version?qualifiers#subpath`).
 * Qualifiers and subpath do not participate in v1 identity matching.
 */
export function parsePurl(value: string): ParsedPurl | undefined {
  if (!value.startsWith('pkg:')) {
    return undefined;
  }
  let rest = value.slice('pkg:'.length);
  rest = rest.split('#', 1)[0].split('?', 1)[0];
  const lastSlash = rest.lastIndexOf('/');
  const at = rest.lastIndexOf('@');
  let version: string | undefined;
  if (at > lastSlash) {
    version = decodeURIComponent(rest.slice(at + 1));
    rest = rest.slice(0, at);
  }
  const segments = rest.split('/');
  if (segments.length < 2 || segments.some(segment => segment.length === 0)) {
    return undefined;
  }
  const [type, ...path] = segments;
  if (!/^[a-zA-Z][a-zA-Z0-9.+-]*$/.test(type)) {
    return undefined;
  }
  return {
    key: `${type.toLowerCase()}/${path.join('/')}`,
    ...(version !== undefined && version.length > 0 ? { version } : {}),
  };
}
