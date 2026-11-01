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
    }
  }
  interface Window {
    showSaveFilePicker?: (options?: { suggestedName?: string }) => Promise<{
      createWritable(): Promise<WritableStream>;
    }>;
  }
}

export {};
