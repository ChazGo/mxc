// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Koffi wrappers for the prototype policy store entry points in mxc_ffi.

import koffi from 'koffi';
import { MxcError } from '../errors.js';
import { loadMxcFfi } from '../native-library.js';
import { bindNativeFunction } from './native-function.js';
import {
  AbiErrorDetailType,
  decodeString,
  nativeStatusError,
  type AbiErrorDetail,
} from './native-error.js';

interface AbiPolicyStoreResult {
  status: number;
  json: unknown | null;
  reason: unknown | null;
  error: AbiErrorDetail;
}

const AbiPolicyStoreResultType = koffi.struct('MxcNodePolicyStoreResult', {
  status: 'int32_t',
  json: 'void *',
  reason: 'void *',
  error: AbiErrorDetailType,
});

type RequestFunction = (request: string, result: AbiPolicyStoreResult) => number;
type InspectFunction = (result: AbiPolicyStoreResult) => number;
type FreeFunction = (result: AbiPolicyStoreResult) => void;

/** The `mxc_ffi` policy store symbols that take a request document. */
export type PolicyStoreRequestSymbol =
  | 'mxc_resolve_sandbox_policy_json'
  | 'mxc_resolve_sandbox_policy_with_diagnostics_json';

/** The `mxc_ffi` policy store symbols that take no input. */
export type PolicyStoreInspectSymbol =
  | 'mxc_policy_catalog_info_json'
  | 'mxc_list_policy_catalog_entries_json';

function decode(status: number, result: AbiPolicyStoreResult): unknown {
  if (status !== 0 || result.status !== 0) {
    const error = nativeStatusError(result.status || status, result.error);
    const reason = decodeString(result.reason);
    if (reason === undefined) throw error;
    throw new MxcError({
      code: error.code,
      message: error.message,
      operation: error.operation,
      nativeCode: error.nativeCode,
      remediation: error.remediation,
      details: { ...error.details, reason },
    });
  }
  const json = decodeString(result.json);
  if (json === undefined) {
    throw new MxcError('backend_error', 'native policy store returned no result');
  }
  return JSON.parse(json);
}

function withResult(
  call: (
    native: ReturnType<typeof loadMxcFfi>,
    result: AbiPolicyStoreResult,
  ) => number,
): unknown {
  const native = loadMxcFfi();
  try {
    const free = bindNativeFunction<FreeFunction>(native.handle, {
      symbol: 'mxc_policy_store_result_free',
      result: 'void',
      parameters: [koffi.pointer(AbiPolicyStoreResultType)],
    });
    const result = {} as AbiPolicyStoreResult;
    let filled = false;
    try {
      const status = call(native, result);
      filled = true;
      return decode(status, result);
    } finally {
      if (filled) free(result);
    }
  } finally {
    native.handle.unload();
  }
}

/** Calls a request-taking policy store entry point and parses its JSON. */
export function callPolicyStore(
  symbol: PolicyStoreRequestSymbol,
  request: unknown,
): unknown {
  return withResult((native, result) =>
    bindNativeFunction<RequestFunction>(native.handle, {
      symbol,
      result: 'int32_t',
      parameters: ['const char *', koffi.out(koffi.pointer(AbiPolicyStoreResultType))],
    })(JSON.stringify(request), result),
  );
}

/** Calls an input-free policy store entry point and parses its JSON. */
export function inspectPolicyStore(symbol: PolicyStoreInspectSymbol): unknown {
  return withResult((native, result) =>
    bindNativeFunction<InspectFunction>(native.handle, {
      symbol,
      result: 'int32_t',
      parameters: [koffi.out(koffi.pointer(AbiPolicyStoreResultType))],
    })(result),
  );
}
