import { fillCredential, generateFill } from "./fill";
import type { FillResult } from "./fill";
import { pickAnchor, showPairingCode, startInline } from "./inline";
import { PAIRING_CODE_EVENT, sendToBackground } from "../lib/messages";
import type { ContentGenerateResult, ContentRequest, SwResponse } from "../lib/messages";

/** A fill belongs to the document it was released for. A tab that navigated
 * to another origin since the request gets nothing, and an inline fill whose
 * picked field is not in this document (`pickAnchor()` is `null`, for example
 * after a same-origin navigation) never falls back to a page-wide fill. */
function fillTarget(
  url: string,
  anchored: boolean,
): { allowed: boolean; anchor: HTMLInputElement | null } {
  let origin: string | null = null;
  try {
    origin = new URL(url).origin;
  } catch {
    origin = null;
  }
  const anchor = pickAnchor();
  return { allowed: origin === location.origin && (!anchored || anchor !== null), anchor };
}

// One copy per frame: the `executeScript` fallback can race the declarative
// injection on a page that is still loading, and a second copy would draw a
// second set of icons and answer every fill twice. The flag lives in this
// isolated world, so the page cannot set or read it.
const scope = globalThis as typeof globalThis & { subclaveContent?: true };

if (!scope.subclaveContent) {
  scope.subclaveContent = true;

  chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    const request = message as ContentRequest;
    switch (request?.type) {
      case "subclave:ping":
        sendResponse({ ok: true });
        return false;
      case "subclave:fill": {
        const { allowed, anchor } = fillTarget(request.url, request.anchored);
        const result: FillResult = allowed
          ? fillCredential(request.username, request.password, anchor)
          : { username: false, password: false };
        sendResponse(result);
        return false;
      }
      case "subclave:generate-fill": {
        const { allowed, anchor } = fillTarget(request.url, request.anchored);
        const result: ContentGenerateResult = allowed
          ? generateFill(request.password, anchor)
          : { username: "", filled: 0 };
        sendResponse(result);
        return false;
      }
      case PAIRING_CODE_EVENT:
        showPairingCode(request.code);
        return false;
      default:
        return false;
    }
  });

  // A page load reads one setting from storage and draws icons; no native
  // connection opens until the user opens the picker, unless the tab has a
  // sign-in waiting for the save prompt.
  const askSettings = (): void => {
    sendToBackground<SwResponse>({ type: "inline-settings" }).then(
      (response) => {
        if (response?.type === "settings" && response.showInLoginFields) startInline();
      },
      () => undefined,
    );
  };
  // A prerendered page's top frame has a non-zero `frameId` until it is shown,
  // and the service worker answers inline requests from frame 0 only, so a
  // prerendered page asks once it is activated.
  const doc: Document & { prerendering?: boolean } = document;
  if (doc.prerendering) {
    document.addEventListener("prerenderingchange", askSettings, { once: true });
  } else {
    askSettings();
  }
}
