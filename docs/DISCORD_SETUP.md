# Discord application setup

Discord only exposes voice state (who is in your channel, who is speaking, who is
muted) to a **registered Discord application**. There is no anonymous API for it,
and this project deliberately does not scrape Discord tokens or read Discord
memory.

In addition, turning a Discord RPC authorization code into an access token
requires the application's **client secret**. Shipping a shared secret inside an
open-source binary would be unsafe and would get revoked, so — like any
independently distributed RPC client — Discord Linux Overlay asks you to create
your own free Discord application once.

This takes about two minutes.

## 1. Create the application

1. Open <https://discord.com/developers/applications> and sign in.
2. **New Application** → pick any name (for example `My Voice Overlay`) → agree
   to the terms → **Create**.
3. On **General Information**, copy the **Application ID**. This is the
   **client id**.

## 2. Copy the client secret

1. Open **OAuth2** in the left sidebar.
2. Under **Client Secret**, click **Reset Secret** (or copy the existing one).
3. Copy the value. This is the **client secret**.

Treat the secret like a password. It is stored only on your machine:

- `$XDG_CONFIG_HOME/discord-linux-overlay/credentials.json` with mode `0600`
- the access token in `discord-token.json` with mode `0600`

Neither file is ever committed, logged or sent anywhere except to
`https://discord.com/api/oauth2/token` for the token exchange.

## 3. Optional: redirect URI

Only set this if you also fill in the *Redirect URI* field in the app:

1. **OAuth2** → **Redirects** → **Add Redirect** → `http://localhost`.
2. Save.

Native Discord RPC clients normally do not need a redirect URI at all; leave the
field empty and no redirect is sent.

## 4. Enter the credentials

1. Start Discord Linux Overlay.
2. Open **Settings → Discord**.
3. Paste the client id and client secret, then **Save credentials**.
4. The overlay connects to the running Discord desktop client and asks you to
   authorize it. Approve the prompt inside Discord (it appears the first time).

The token is cached, so the prompt normally appears only once. If Discord
revokes it, the overlay asks again automatically.

## Requirements and expectations

- **Official Discord desktop client only.** Vesktop, ArmCord, WebCord and other
  third-party clients do not implement Discord's RPC voice API. Voice detection
  will stay empty with them.
- Discord must be running before (or after) the overlay; the overlay keeps
  retrying and reports "Waiting for the Discord desktop client".
- The scopes requested are `identify` and `rpc`. Nothing is written to Discord.

## Troubleshooting

| Symptom | Meaning / fix |
| --- | --- |
| "Setup required" | Client id or secret missing. Re-enter them. |
| "Discord refused the application authorization (4007)" | The client id is wrong or belongs to a deleted application. |
| "Discord rejected the cached token" | Normal after a revocation; approve the new prompt. |
| Authorization prompt never appears | Discord may be minimized to the tray; open it. Also check you are not using a third-party client. |
| The overlay connects but you are not in a voice channel | Expected — join a voice channel; the overlay updates instantly. |
| Token exchange fails with HTTP 401 | The secret was reset after you copied it. Copy the current secret again. |
