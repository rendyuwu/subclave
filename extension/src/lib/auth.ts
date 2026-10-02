// The pairing code and the HMAC handshake, mirrored from
// `src-tauri/src/modules/browser/auth.rs`. Pure WebCrypto: no chrome API, so
// `extension/test/auth.spec.ts` can import this module directly and the pinned
// vectors prove the two implementations agree.
//
//   appProof = HMAC-SHA256(secret, "subclave-app-v1" || extNonce || appNonce)
//   extProof = HMAC-SHA256(secret, "subclave-ext-v1" || appNonce || extNonce)

export const PAIR_CONTEXT = "subclave-pair-v1";
export const APP_CONTEXT = "subclave-app-v1";
export const EXT_CONTEXT = "subclave-ext-v1";

/** `Uint8Array` pinned to an `ArrayBuffer` so it is a valid `BufferSource`. */
export type Bytes = Uint8Array<ArrayBuffer>;

const encoder = new TextEncoder();

export function concatBytes(...parts: Bytes[]): Bytes {
  const total = parts.reduce((sum, part) => sum + part.length, 0);
  const out = new Uint8Array(total);
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

export function bytesToBase64(bytes: Bytes): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

export function base64ToBytes(value: string): Bytes {
  const binary = atob(value);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) out[i] = binary.charCodeAt(i);
  return out;
}

/** 32 bytes from the platform CSPRNG. */
export function randomNonce(): Bytes {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return bytes;
}

/**
 * First 4 bytes of SHA-256("subclave-pair-v1" || pairNonce) read as a big-endian
 * u32, taken modulo 1,000,000 and zero-padded to six digits.
 */
export async function pairingCode(pairNonce: Bytes): Promise<string> {
  const digest = new Uint8Array(
    await crypto.subtle.digest("SHA-256", concatBytes(encoder.encode(PAIR_CONTEXT), pairNonce)),
  );
  const value = new DataView(digest.buffer).getUint32(0, false);
  return String(value % 1_000_000).padStart(6, "0");
}

async function hmacKey(secret: Bytes): Promise<CryptoKey> {
  return crypto.subtle.importKey(
    "raw",
    secret,
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign", "verify"],
  );
}

export async function appProof(secret: Bytes, extNonce: Bytes, appNonce: Bytes): Promise<Bytes> {
  const message = concatBytes(encoder.encode(APP_CONTEXT), extNonce, appNonce);
  return new Uint8Array(await crypto.subtle.sign("HMAC", await hmacKey(secret), message));
}

export async function extProof(secret: Bytes, appNonce: Bytes, extNonce: Bytes): Promise<Bytes> {
  const message = concatBytes(encoder.encode(EXT_CONTEXT), appNonce, extNonce);
  return new Uint8Array(await crypto.subtle.sign("HMAC", await hmacKey(secret), message));
}

/** HMAC verification through WebCrypto, which is constant time in the browser. */
export async function verifyProof(
  secret: Bytes,
  message: Bytes,
  signature: Bytes,
): Promise<boolean> {
  return crypto.subtle.verify("HMAC", await hmacKey(secret), signature, message);
}

export function verifyAppProof(
  secret: Bytes,
  extNonce: Bytes,
  appNonce: Bytes,
  signature: Bytes,
): Promise<boolean> {
  return verifyProof(secret, concatBytes(encoder.encode(APP_CONTEXT), extNonce, appNonce), signature);
}

export function verifyExtProof(
  secret: Bytes,
  appNonce: Bytes,
  extNonce: Bytes,
  signature: Bytes,
): Promise<boolean> {
  return verifyProof(secret, concatBytes(encoder.encode(EXT_CONTEXT), appNonce, extNonce), signature);
}
