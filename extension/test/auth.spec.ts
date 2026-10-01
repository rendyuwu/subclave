import { expect, test } from "@playwright/test";
import { appProof, extProof, pairingCode } from "../src/lib/auth";

// The three pinned constants are the same literals the Rust test in
// `src-tauri/src/modules/browser/auth.rs` asserts, so a drift between the two
// implementations reddens both suites.

const hex = (bytes: Uint8Array): string =>
  [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");

test("pairingCode pins the shared vector", async () => {
  expect(await pairingCode(new Uint8Array(32))).toBe("645339");
});

test("the HMAC handshake pins the Rust vectors", async () => {
  const secret = new Uint8Array(32).fill(1);
  const extNonce = new Uint8Array(32).fill(2);
  const appNonce = new Uint8Array(32).fill(3);

  expect(hex(await appProof(secret, extNonce, appNonce))).toBe(
    "eabf7b81f143227129cca156b66e04d3d46c2f24b605c595b5f3062b858e5d8e",
  );
  expect(hex(await extProof(secret, appNonce, extNonce))).toBe(
    "e82b14c434f1915828d86889b56358cea216b521862ee312595f251f9ed7d159",
  );
});
