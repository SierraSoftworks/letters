# Operating a Letters server

`letters-server` is one program with one settings file. Its documents are plain `.let` files in the
folders the settings name as spaces, and everything else it remembers is in one state directory. To reach
it over Tailscale, see [tailscale.md](tailscale.md).

## Settings

The file is TOML, read once at start: `--config <file>`, else `LETTERS_CONFIG`, else
`/etc/letters/server.toml`. A key the server does not know is an error, so a misspelling cannot leave a
guard off.

```sh
letters-server --config /etc/letters/server.toml --check
```

`--check` reads the settings and the secret file, says what is wrong and exits: 0 when sound, 2 when not.

### The server and its sign-in

| Key | Default | Says |
| --- | --- | --- |
| `server.url` | required | The address people reach it at. `https://` and a host, no path. `http://` only for this machine. |
| `server.state` | required | The state directory, on a local disk. |
| `server.name` | `"Letters"` | What the pages call the server. |
| `server.listen` | `127.0.0.1:8080` | Where it listens. |
| `server.proxies` | `["127.0.0.1", "::1"]` | The proxies whose `X-Forwarded-For` is believed. |
| `server.log` | `"info"` | `"debug"` adds a line for every request. |
| `auth.session_days` | `7` | How long a sign-in lasts, and so how long a removed group still counts. |
| `auth.oidc.issuer` | required | The identity provider's address. |
| `auth.oidc.client_id` | required | The client the provider knows the server as. |
| `auth.oidc.client_secret` | none | The client's secret, or |
| `auth.oidc.client_secret_file` | none | a file that holds it. One of the two at most. |
| `auth.oidc.username_claim` | `"preferred_username"` | Names a person's own space. |
| `auth.oidc.name_claim` | `"name"` | The name a person is shown by. |
| `auth.oidc.groups_claim` | `"groups"` | Lists a person's groups. |
| `auth.oidc.scopes` | `["openid", "profile", "email"]` | Must hold `openid`. |

### Spaces

Each `[[space]]` is a folder of documents. There must be at least one.

- **`name`, `path`.** The name shows in the pages, and its lower-case form names the space in addresses.
  The path starts at the root. One that holds `{username}` is each person's own space, made at their first
  sign-in, and takes no groups.
- **`owners`, `editors`, `commenters`, `readers`.** Lists of groups from the sign-in. A shared space names
  at least one. The highest role a person's groups give them counts.
- **Owners.** They edit, and they alone move a folder to the trash. Editors rename and move folders, and
  move documents to the trash.
- **What is left out.** Names with a leading dot, and the folders a NAS or a disk keeps for itself
  (`@eaDir`, `#recycle`, `#snapshot`, `$RECYCLE.BIN`, `lost+found` and their like), are no part of a space.

### Switches

| Key | Default | Says |
| --- | --- | --- |
| `links.enabled` | `false` | Whether documents can be shared by link. See "Share links" in the Tailscale guide. |
| `trust.sources` | `[]` | Where the server may fetch pictures and link previews from, for everyone. |
| `telemetry.enabled` | `true` | Whether a release build reports that it runs and which features are used. Nothing of a document, a person or an address is sent. `DO_NOT_TRACK=1` in the environment turns it off too. |

## Keeping it safe

Losing the state directory loses no document. Losing a space loses its documents.

### What holds what

- **Spaces.** Each `.let` is whole by itself: text, pictures, comments and history. A space also holds
  `.letters-trash/` (its trash) and, in a person's own space, `.letters/` (their dictionary and default
  style).
- **State directory.** `letters.redb` holds shares, links, the access log, sign-ins, what each person set
  and trusted, and the index of documents. `rooms/` holds the journals of open documents: what was typed
  since each was last saved. `dictionaries/` and `fallback/` hold what the owner put there.
- **Settings.** The settings file and the secret file are nowhere else.

### Backup and restore

- **Spaces.** Copy them at any time. A save replaces a file whole, so a copy never holds half of one. What
  is being typed reaches its file two seconds after a pause, and within 30 seconds at most.
- **State.** Stop the server first: it saves every open document as it stops, and the store is one file
  that must not be copied while it is written.
- **Restore.** Put the folders back where the settings name them and start. The server reads what its
  journals hold into the files, then scans the spaces and rebuilds its index. With no state directory it
  starts empty: every document is there, everyone signs in again, and shares, links, the access log and
  people's own settings are gone, so nothing stays shared that was.

### Upgrade

- **Steps.** Stop, replace the program, run `--check`, start.
- **The store's mark.** The store says which layout it was written in (now 1). A build that finds a
  later layout than it knows refuses to start, says so and leaves the file as it is.
- **Going back.** An older build runs on a store whose mark it knows. Otherwise restore the state
  directory from before the upgrade, or start with an empty one.

## Watching and tidying

### Log and health

- **Lines.** On standard error: time (RFC 3339, UTC), kind, text. `info` is the server's own course,
  `warn` is something that went wrong and that it got past.

  ```
  2026-11-16T08:00:00Z info  listening on 127.0.0.1:8080 as https://letters.example.ts.net
  ```

- **`log = "debug"`.** One more line a request: method, route, status, milliseconds. The route is the
  pattern, so no document id, link secret, file name or person is written.
- **`GET /health`.** `200 ok` while the store answers, `503` when it does not. No sign-in is needed.

### Limits

These are fixed in the program. Only `auth.session_days` is a setting.

- **Sizes.** A document 256 MiB, a picture 32 MiB, a message on a document's socket 8 MiB, a dictionary
  file 64 MiB.
- **Links.** 50 live links a document. A visit lasts 12 hours or until the link ends. One address
  follows links 30 times a minute, and a passphrase try counts as 6.
- **Access log.** A row is kept 365 days. The same thing by the same reader within 5 minutes is one row.
- **Exports.** The server writes two at a time as PDF, Word, Markdown or a web page, and the rest wait
  their turn. A link's visit is written 5 a minute.

### What the owner cleans

The server deletes nothing a person made for good.

- **Trash.** What people throw away moves to `.letters-trash/` of its space and stays. Delete there to
  empty it.
- **Set-aside journals.** A journal with a damaged edit is taken as far as it stands, logged at `warn`
  and kept in `rooms/` as a file with `set-aside` in its name, which the log line names with its
  document. It holds whatever the damage cost. Delete it once nobody misses an edit.
- **`dictionaries/` and `fallback/`.** Hunspell pairs such as `de_DE.aff` and `de_DE.dic` serve spelling
  in other languages to the browser. `fallback/` takes the faces of the web build's `dist/fallback/`, for
  emoji and scripts the built-in fonts lack, in the browser's editor and in a PDF the server writes. Both
  are read as they are asked for, with no restart.
