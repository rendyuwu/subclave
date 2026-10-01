import { startBackground } from "../../src/background";
import { createFakeTransport } from "./fake-app";

// The service worker the E2E builds ship: the production background logic with
// the native port replaced by the fake app. Nothing else about it changes, so
// the specs exercise the real router, handshake and fill paths.
startBackground({ transport: createFakeTransport() });
