// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { execFileSync } from 'node:child_process';
import * as os from 'node:os';
import * as path from 'node:path';
import { PolicyCatalogError } from './errors.js';
import type { CatalogArchitecture, CatalogPlatform } from './types.js';

/** Host facts the resolver uses only when the caller omits them. */
export interface HostEnvironment {
  platform(): CatalogPlatform;
  /**
   * The device's native system architecture (design §4.4). This is not the
   * architecture of the process hosting the library, and not a detected tool
   * build. Throw when it cannot be determined; never guess.
   */
  nativeArchitecture(): CatalogArchitecture;
  /** Approved host-known symbols (`source: "host"` in the contract) for the current host. */
  symbol(name: string): string | undefined;
}

function hostPlatform(): CatalogPlatform {
  switch (os.platform()) {
    case 'win32':
      return 'windows';
    case 'darwin':
      return 'macos';
    case 'linux':
      return 'linux';
    default:
      throw new PolicyCatalogError('unsupported_host', `host platform '${os.platform()}' has no catalog selector`);
  }
}

/** Maps an OS-reported machine/architecture string to a catalog selector. */
export function architectureFromMachine(machine: string): CatalogArchitecture | undefined {
  switch (machine.trim().toLowerCase()) {
    case 'x86_64':
    case 'amd64':
    case 'x64':
      return 'x64';
    case 'arm64':
    case 'aarch64':
      return 'arm64';
    default:
      return undefined;
  }
}

function run(file: string, args: string[]): string {
  return execFileSync(file, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'], windowsHide: true });
}

/**
 * Reads the native architecture without trusting the current process's view.
 *
 * - Windows: an x64 process emulated on ARM64 sees `PROCESSOR_ARCHITECTURE=AMD64`
 *   and an x64 `os.machine()`. The machine-wide value in the registry reports
 *   the native architecture.
 * - macOS: a Rosetta-translated process sees `x86_64` from `uname`.
 *   `hw.optional.arm64` reports Apple silicon regardless of translation, and
 *   Intel Macs do not define it.
 * - Linux: `uname` reports the kernel's machine type.
 *
 * Executables are invoked by absolute path so PATH cannot redirect them.
 */
function detectNativeArchitecture(): CatalogArchitecture {
  const platform = os.platform();
  let reported: string | undefined;
  try {
    if (platform === 'win32') {
      const reg = path.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'reg.exe');
      const output = run(reg, [
        'query',
        'HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment',
        '/v',
        'PROCESSOR_ARCHITECTURE',
      ]);
      reported = /PROCESSOR_ARCHITECTURE\s+REG_SZ\s+(\S+)/i.exec(output)?.[1];
    } else if (platform === 'darwin') {
      let appleSilicon = false;
      try {
        appleSilicon = run('/usr/sbin/sysctl', ['-n', 'hw.optional.arm64']).trim() === '1';
      } catch {
        // The key is absent on Intel Macs.
      }
      reported = appleSilicon ? 'arm64' : os.machine();
    } else {
      reported = os.machine();
    }
  } catch (error) {
    throw new PolicyCatalogError('unsupported_host', `native system architecture could not be determined: ${(error as Error).message}`);
  }
  const architecture = reported === undefined ? undefined : architectureFromMachine(reported);
  if (!architecture) {
    throw new PolicyCatalogError('unsupported_host', `native system architecture '${reported ?? 'unknown'}' has no catalog selector`);
  }
  return architecture;
}

let nativeArchitectureCache: CatalogArchitecture | undefined;

/** Default host environment. The native architecture is detected once per process, on first need. */
export const nodeHostEnvironment: HostEnvironment = {
  platform: hostPlatform,
  nativeArchitecture() {
    nativeArchitectureCache ??= detectNativeArchitecture();
    return nativeArchitectureCache;
  },
  symbol(name) {
    switch (name) {
      case 'user_home':
        return os.homedir() || undefined;
      case 'temp_dir':
        return os.tmpdir() || undefined;
      default:
        return undefined;
    }
  },
};
