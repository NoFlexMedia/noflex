// SPDX-License-Identifier: AGPL-3.0-only

declare global {
  namespace App {
    interface GraphQLErrorExtensions {
      code?: unknown;
      /** Set by the client for a transport failure: the original error. */
      error?: globalThis.Error;
    }
    interface Metadata {
      /** Don't refresh the access token on an auth error: sign-in and other unauthenticated calls. */
      noRefresh?: boolean;
      /** Send to this page's own server even while a linked one is active. */
      ownServer?: boolean;
    }
    interface PageState {
      /** Marks the history entry owned by the open player. */
      player?: number;
    }
  }
  interface Window {
    showSaveFilePicker?: (options?: { suggestedName?: string }) => Promise<{
      createWritable(): Promise<WritableStream>;
    }>;
  }
}

export {};
