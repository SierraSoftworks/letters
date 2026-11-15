# Letters on a tailnet

Tailscale carries the server to its people and says who they are. `tailscale serve` gives the server its
address and certificate, and [tsidp](https://github.com/tailscale/tsidp), Tailscale's identity provider, signs
a person in as the owner of the device they are on, with no password. Nothing is open to the internet.

The examples use the tailnet `example.ts.net`, a server machine named `letters`, and the tailnet groups
`family` and `friends`. The guide was written against tsidp v0.0.15. tsidp is experimental, and the shape of
its grants may change.

## Publish

- **Tailnet.** Turn on MagicDNS and HTTPS certificates in the admin console, and let the provider carry a
  tag: `"tagOwners": { "tag:tsidp": ["autogroup:admin"] }`.
- **Provider.** Run tsidp as `idp.example.ts.net`, with an auth key for the tag and a volume that outlasts
  the container:

  ```yaml
  services:
    tsidp:
      image: ghcr.io/tailscale/tsidp:latest
      volumes: ["tsidp-data:/data"]
      environment:
        - TAILSCALE_USE_WIP_CODE=1
        - TS_STATE_DIR=/data
        - TS_HOSTNAME=idp
        - TS_ADVERTISE_TAGS=tag:tsidp
        - TS_AUTHKEY=${TS_AUTHKEY}
  volumes:
    tsidp-data:
  ```

- **Letters.** The server listens on `127.0.0.1:8080`, and `tailscale serve --bg 8080` on its machine
  answers for it at `https://letters.example.ts.net`.

## Register

- **Admin page.** A grant in the tailnet policy lets its admins open `https://idp.example.ts.net`:

  ```hujson
  {
    "src": ["autogroup:admin"],
    "dst": ["tag:tsidp"],
    "ip":  ["tcp:443"],
    "app": { "tailscale.com/cap/tsidp": [{ "allow_admin_ui": true }] },
  },
  ```

- **Client.** "Create Client" there, with the redirect address `https://letters.example.ts.net/auth/callback`.
- **Secret.** The page shows the client's id and its secret once. The secret goes in a file that only the
  server reads, such as `/etc/letters/oidc-secret`.

## Map groups

- **Grants.** One grant for each tailnet group puts the group's name in the `groups` claim of its members.
  Someone in several groups carries each name.

  ```hujson
  {
    "src": ["group:family"],
    "dst": ["tag:tsidp"],
    "ip":  ["tcp:443"],
    "app": {
      "tailscale.com/cap/tsidp": [{ "extraClaims": { "groups": ["family"] }, "includeInUserInfo": true }],
    },
  },
  ```

- **Spaces.** A space names those groups as its `editors`, `commenters` or `readers`. A space whose path
  holds `{username}` is each person's own.
- **People.** A person signs in from a device of their own: tsidp turns a tagged device away. Their groups
  are read when they sign in, so a change in the policy reaches them at their next sign-in, `session_days`
  later at most.

## Settings

```toml
[server]
url = "https://letters.example.ts.net"
state = "/var/lib/letters"

[auth.oidc]
issuer = "https://idp.example.ts.net"
client_id = "the id the provider showed"
client_secret_file = "/etc/letters/oidc-secret"

[[space]]
name = "Home"
path = "/srv/letters/home/{username}"

[[space]]
name = "Family"
path = "/srv/letters/family"
editors = ["family"]
readers = ["friends"]
```

- **Claims.** The claims the server reads by default are the ones tsidp tells: `preferred_username` is a
  person's login up to its `@`, and `name` is their name in the tailnet. A tailnet whose logins end in more
  than one domain sets `username_claim = "email"`, so that two people never share a folder.
- **Reach.** Everyone who signs in reaches both machines on port 443, and the server's machine reaches the
  provider. A tailnet's first policy allows all of it, and a stricter one grants it.
- **Check.** `letters-server --check` reads the settings and says what is wrong with them.
