import { SyncField } from "@/settings/components/SyncField";
import type { JoinCredentials } from "./useJoinSync";

/**
 * The credential pair the selected provider reads: a WebDAV username and
 * password, or an S3 access key and secret. The other pair is not rendered at
 * all, so a value can never be typed into the wrong one.
 */
export function JoinCredentialFields({
  provider,
  credentials,
  onChange,
}: {
  provider: string;
  credentials: JoinCredentials;
  onChange: (credentials: JoinCredentials) => void;
}) {
  const webdav = provider === "webdav";

  return webdav ? (
    <>
      <SyncField
        id="join-webdav-username"
        label="Username"
        description="The account on the WebDAV server. Stored only inside this vault, encrypted with the master password below."
        autoComplete="off"
        value={credentials.username}
        onChange={(e) => onChange({ ...credentials, username: e.target.value })}
      />
      <SyncField
        id="join-webdav-password"
        label="Password"
        description="Stored only inside this vault, encrypted with the master password below. If your server offers app passwords, one of those is worth more here than your account password: it can be revoked on its own."
        type="password"
        autoComplete="off"
        value={credentials.webdavPassword}
        onChange={(e) => onChange({ ...credentials, webdavPassword: e.target.value })}
      />
    </>
  ) : (
    <>
      <SyncField
        id="join-access-key-id"
        label="Access key ID"
        description="Stored only inside this vault, encrypted with the master password below."
        autoComplete="off"
        value={credentials.accessKeyId}
        onChange={(e) => onChange({ ...credentials, accessKeyId: e.target.value })}
      />
      <SyncField
        id="join-secret-access-key"
        label="Secret access key"
        description="Stored only inside this vault, encrypted with the master password below."
        type="password"
        autoComplete="off"
        value={credentials.secretAccessKey}
        onChange={(e) => onChange({ ...credentials, secretAccessKey: e.target.value })}
      />
    </>
  );
}
