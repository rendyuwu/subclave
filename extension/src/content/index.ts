import { fillCredential, generateFill } from "./fill";
import type { ContentGenerateResult, ContentRequest } from "../lib/messages";
import type { FillResult } from "./fill";

// No page-load work yet: field detection, the inline icon and the picker are
// not built, so a page load opens no native connection and draws nothing.
chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  const request = message as ContentRequest;
  switch (request?.type) {
    case "subclave:ping":
      sendResponse({ ok: true });
      return false;
    case "subclave:fill": {
      const result: FillResult = fillCredential(request.username, request.password);
      sendResponse(result);
      return false;
    }
    case "subclave:generate-fill": {
      const result: ContentGenerateResult = generateFill(request.password);
      sendResponse(result);
      return false;
    }
    default:
      return false;
  }
});
