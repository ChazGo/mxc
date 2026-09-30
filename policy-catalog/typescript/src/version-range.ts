// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * Minimal, dependency-free version-range support for catalog v1.
 *
 * Grammar (a deliberate subset of npm semver ranges, identical in every
 * language binding):
 *
 *   range      := comparatorSet ( "||" comparatorSet )*
 *   comparatorSet := comparator ( " " comparator )*
 *   comparator := op? version
 *   op         := ">=" | "<=" | ">" | "<" | "="
 *   version    := N ( "." N ( "." N )? )?
 *
 * Missing minor/patch components are zero-filled. A comparator with no
 * operator is a prefix match (`22` means `>=22.0.0 <23.0.0`). Prerelease and
 * build metadata on evidence versions are ignored.
 */

type Triple = [number, number, number];
type Op = '>=' | '<=' | '>' | '<' | '=' | 'prefix';
interface Comparator {
  op: Op;
  version: Triple;
  parts: number;
}

const COMPARATOR = /^(>=|<=|>|<|=)?(\d+)(?:\.(\d+)(?:\.(\d+))?)?$/;

function parseComparator(token: string): Comparator | undefined {
  const match = COMPARATOR.exec(token);
  if (!match) {
    return undefined;
  }
  const parts = match[4] !== undefined ? 3 : match[3] !== undefined ? 2 : 1;
  return {
    op: (match[1] as Op | undefined) ?? 'prefix',
    version: [Number(match[2]), Number(match[3] ?? 0), Number(match[4] ?? 0)],
    parts,
  };
}

function parseRange(range: string): Comparator[][] | undefined {
  if (range.trim().length === 0) {
    return undefined;
  }
  const sets: Comparator[][] = [];
  for (const alternative of range.split('||')) {
    const tokens = alternative.trim().split(/\s+/).filter(Boolean);
    if (tokens.length === 0) {
      return undefined;
    }
    const comparators: Comparator[] = [];
    for (const token of tokens) {
      const comparator = parseComparator(token);
      if (!comparator) {
        return undefined;
      }
      comparators.push(comparator);
    }
    sets.push(comparators);
  }
  return sets;
}

/** Returns true when `range` uses the supported v1 range grammar. */
export function isValidVersionRange(range: string): boolean {
  return parseRange(range) !== undefined;
}

/** Parses tool version evidence such as `v22.3.1` or `10.9.0-rc.1`. */
export function parseVersionEvidence(value: string): Triple | undefined {
  const match = /^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:[-+].*)?$/.exec(value.trim());
  if (!match) {
    return undefined;
  }
  return [Number(match[1]), Number(match[2] ?? 0), Number(match[3] ?? 0)];
}

function compare(left: Triple, right: Triple): number {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) {
      return left[index] < right[index] ? -1 : 1;
    }
  }
  return 0;
}

function satisfiesComparator(version: Triple, comparator: Comparator): boolean {
  const order = compare(version, comparator.version);
  switch (comparator.op) {
    case '>=':
      return order >= 0;
    case '<=':
      return order <= 0;
    case '>':
      return order > 0;
    case '<':
      return order < 0;
    case '=':
      return order === 0;
    case 'prefix':
      return version.slice(0, comparator.parts).every((part, index) => part === comparator.version[index]);
  }
}

/**
 * Evaluates `version` against `range`. Returns `undefined` when either value
 * cannot be evaluated; callers treat that as "not verified", never a match.
 */
export function satisfiesVersionRange(version: string, range: string): boolean | undefined {
  const parsedVersion = parseVersionEvidence(version);
  const parsedRange = parseRange(range);
  if (!parsedVersion || !parsedRange) {
    return undefined;
  }
  return parsedRange.some(set => set.every(comparator => satisfiesComparator(parsedVersion, comparator)));
}
