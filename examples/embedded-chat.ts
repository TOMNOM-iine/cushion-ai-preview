/**
 * Native integration example, not a server or an external API client.
 * Supply only Tauri's local invoke in a trusted, locally bundled desktop UI.
 * The native Core remains the authority; isNative is not an authentication check.
 */
import { createCushionClient } from '../packages/cushion-sdk/src/index';
import type { CoreRequest, Dispatch, LocalOnlyHost, Snapshot } from '../packages/cushion-sdk/src/contracts';

export interface LocalInvoke {
  (command: 'dispatch', args: { request: CoreRequest }): Promise<Snapshot>;
  (command: 'api_send', args: { approvalId: string }): Promise<Snapshot>;
}

export function nativePorts(invokeLocal: LocalInvoke): {
  dispatch: Dispatch; localOnlyHost: LocalOnlyHost;
} {
  // Serialize local edits and reads. The long native delivery is kept separate;
  // the Rust host owns its export transaction and the SDK rejects stale results.
  let tail: Promise<unknown> = Promise.resolve();
  const dispatch: Dispatch = request => {
    const task = tail.then(() => invokeLocal('dispatch', { request }));
    tail = task.catch(() => undefined);
    return task;
  };
  return {
    dispatch,
    localOnlyHost: {
      isNative: true,
      // Construct a fresh object. No text, dictionary, mapping, model, endpoint,
      // API key or restored answer can be supplied to this command by this seam.
      sendApproved: approvalId => invokeLocal('api_send', { approvalId }),
    },
  };
}

export function connectNativeChat(invokeLocal: LocalInvoke) {
  // The caller first starts a consultation and establishes mode: 'local' through
  // native Core IPC. This factory neither resets existing work nor grants trust.
  return createCushionClient(nativePorts(invokeLocal));
}

/** This works now with the included Rust WASM and its frozen fictional samples. */
export function connectOfflineDemo(dispatch: Dispatch) {
  return createCushionClient({ dispatch });
}
