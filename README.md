# TeamsCLI

A terminal UI client for **Microsoft Teams**, written in Rust. Sign in from your
terminal with a browser-based device code, then browse and reply to your chats —
all without leaving the keyboard.

> **Status: Phase 1 (MVP).** Device-code sign-in + 1:1 and group chat read/send.
> Team channels, presence, and notifications are on the roadmap below.

---

## How it works (and the trade-offs)

TeamsCLI talks to Teams' **internal API** — the same endpoints the official Teams
client uses — rather than the Microsoft Graph API. It authenticates as the
**official Microsoft Teams first-party application**, which is pre-consented in
every tenant.

**Why this approach:** the Graph API route requires registering your own Azure AD
app and, in most organizations, an administrator to approve it. Many users can't
get that approval. Authenticating as the first-party Teams client sidesteps it
entirely — **no app registration, no admin consent.**

**The honest caveats:**

- These endpoints are **undocumented**. Microsoft can change or break them at any
  time, without notice.
- It's a **gray area** with respect to the Teams terms of service (you're
  automating against a first-party client). You are accessing *your own* account.
- Expect occasional breakage and field-shape changes between accounts/regions.

If your organization *can* grant admin consent and you want a fully sanctioned,
stable integration, the Microsoft Graph API is the better long-term choice — but
that's not what this build uses.

### The flow

1. **Device-code sign-in** as the Teams first-party client for the
   `https://api.spaces.skype.com` resource → an AAD access + refresh token.
2. The access token is exchanged at `teams.microsoft.com/api/authsvc/v1.0/authz`
   for a **skypetoken** and your region's messaging host.
3. Chats are listed from the chat-service aggregator; messages are read/sent on
   the region-specific messaging host. The refresh token is cached locally for
   silent re-login.

---

## Build & run

Requires a recent stable Rust toolchain. **No setup or registration needed.**

```sh
cargo build --release
./target/release/teamscli
```

On first launch you'll see:

```
  To sign in, open: https://login.microsoft.com/device
  And enter the code: ABCD-EFGH

  Waiting for authorization…
```

Authorize in your browser; the TUI then loads your chats. Your refresh token is
cached, so subsequent launches sign you in silently.

> Works with **work/school accounts**. (Personal Microsoft accounts have very
> limited Teams chat access on these endpoints.)

---

## Command-line options

```
teamscli [OPTIONS]
teamscli doctor [OPTIONS]

  -h, --help               Show help and exit
      --version            Show version and exit
      --msg <N>            Messages to load per conversation (default 50)
      --log-level <LEVEL>  trace | debug | info | warn | error (default info)
      --debug              Shortcut for --log-level debug
      --refresh <SECS>     Background poll interval in seconds (default 4)
      --no-live            Disable background polling entirely
```

`--log-level` (and `--debug`) take precedence over the `TEAMSCLI_LOG` env var.

### `doctor`

If sign-in or loading misbehaves, run the built-in diagnostics:

```sh
teamscli doctor
```

It checks the build/terminal, that the log directory is writable, the token
cache (presence + `0600` permissions), TCP reachability of the Teams endpoints,
and validates your cached credentials with a live silent refresh — then prints a
`PASS`/`WARN`/`FAIL` summary and exits non-zero if anything failed.

---

## Keybindings

Navigation is vim-style and acts on the **focused pane** (chat list ↔ messages).

| Key | Action |
|-----|--------|
| `Tab` / `h` / `l` | Switch focus between chat list and messages |
| `Ctrl-B` | Toggle the chat list (full-width message reading) |
| `j` / `k` | Down / up in the focused pane (selecting a chat opens it) |
| `Ctrl-D` / `Ctrl-U` | Half-page down / up |
| `gg` / `G` | Jump to top / bottom |
| mouse wheel | Scroll the focused pane |
| `Enter` | Open the chat and focus the message view |
| `/` | Search / filter the chat list |
| `i` | Compose a message |
| `Enter` (compose) | Send the message |
| `Esc` | Leave compose / clear filter / back to chat list |
| `?` | Toggle help |
| `q` / `Ctrl-C` | Quit |

---

## Files & logging

| What | Location |
|------|----------|
| Token cache | `${XDG_CONFIG_HOME:-~/.config}/teamscli/token.json` (`0600`) |
| Logs | `${XDG_DATA_HOME:-~/.local/share}/teamscli/logs/teamscli.log.*` |

The TUI owns stdout, so all diagnostics go to the log file. Increase verbosity
with `TEAMSCLI_LOG=debug`. To sign out, delete the token cache file.

---

## Architecture

```
src/
├── main.rs        # entry, terminal lifecycle, async event loop
├── cli.rs         # argument parsing (flags + `doctor` subcommand)
├── doctor.rs      # preflight diagnostics (tokens, network, config)
├── config.rs      # endpoints, first-party client ID, XDG paths
├── auth/          # device-code flow (v1) + token cache / silent refresh
├── teams/         # internal API client
│   ├── mod.rs     #   token mgmt + host-based auth-header routing + retry
│   ├── authz.rs   #   skypetoken exchange + region messaging host
│   ├── conversations.rs  # list chats (CSA)
│   ├── messages.rs       # read / send messages (messaging host)
│   └── models.rs  #   public ChatSummary/Message + permissive raw JSON shapes
├── app/           # UI-agnostic state machine + async fetch helpers (poller)
├── ui/            # ratatui rendering (two-pane layout + status bar)
├── event.rs       # unified event enum funneled through one mpsc channel
├── notify.rs      # desktop notifications (wired up in a later phase)
└── util.rs        # JWT-claim decode (display name) + HTML→text
```

Real-time updates use **polling** (the active chat is re-fetched every few
seconds), since these endpoints don't offer a practical local push channel.

---

## Troubleshooting

- **Sign-in works but chats don't load / look wrong:** these endpoints are
  undocumented and field shapes vary by account and region. Check the log file
  (`TEAMSCLI_LOG=debug`) — the raw error/URL is recorded there. The chat-list
  field mapping in `src/teams/models.rs` is the most likely thing to need a tweak
  against a real account.
- **`skypetoken exchange failed`:** the AAD token didn't have the expected
  audience; re-run sign-in after deleting the token cache.
- **Personal account:** most Teams chat endpoints require a work/school account.

---

## Roadmap

- **Phase 2 — Team channels:** browse teams/channels, read & post.
- **Phase 3 — Presence & status.**
- **Phase 4 — Notifications:** alerts and unread badges for unfocused chats.

---

## Development

```sh
cargo fmt
cargo clippy
cargo test
```

## Disclaimer

This project uses unofficial, undocumented Microsoft Teams endpoints and is not
affiliated with or endorsed by Microsoft. Use it with your own account and at
your own risk.

## License

MIT
