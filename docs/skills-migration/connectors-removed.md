# Migration: connector packs removed from apxm-libs

On 2026-06-03 the seven `apxm-app-*` connector packs were removed from
apxm-libs. apxm-libs is now a **pure operating-skill library** (one content
kind). This file records why, what the connectors held, and where that data
belongs.

## Why they were removed

A connector pack duplicated data that **`apxm-auth/providers.toml` already
owns**. For every provider, `providers.toml` already declares `display_name`,
`category`, `api_base`, `auth_kind`, `auth_inject`, `webhook_scheme`, scopes,
and a `test_capability` — and it is already parsed by `apxm-studio` and
`apxm-os` (the file header says so). The connector pack re-declared:

- `category` / `icon` → already in `providers.toml`.
- the action `url` → re-baked `api_base` (already in `providers.toml`) into each
  per-action URL.
- the `capability` → `providers.toml` already names a `test_capability`.

The only genuinely *new* data in a connector pack was the per-action
`method` + URL-suffix table and the trigger cues. That is **provider
integration metadata**, which belongs next to the provider in `apxm-auth`, not
in a skill library. Keeping it in apxm-libs created two divergent sources of
truth (the studio "two-lists" hazard) and made the skill library incoherent.

The earlier redesign kept connectors on the claim that they had "no other home
in the stack." That was wrong: `apxm-auth` is the home and already holds half
the descriptor.

## What each connector contributed (preserved here verbatim)

These are the action/trigger descriptors to fold into the matching provider in
`apxm-auth/providers.toml`. `api_base` is already in `providers.toml`; record
the **method + path suffix + capability** (and `read_only`), not the full URL.

| Provider | capability | method | url | read_only |
|---|---|---|---|---|
| discord | `discord.post` | POST | `https://discord.com/api/v10/channels/{channel_id}/messages` | false |
| slack | `slack.post` | POST | `https://slack.com/api/chat.postMessage` | false |
| telegram | `telegram.send` | POST | `https://api.telegram.org/bot{token}/sendMessage` | false |
| github | `github.issue_create` | POST | `https://api.github.com/repos/{owner}/{repo}/issues` | false |
| notion | `notion.search` | POST | `https://api.notion.com/v1/search` | true |
| notion | `notion.create_page` | POST | `https://api.notion.com/v1/pages` | false |
| x | `social.x.post` | POST | `https://api.x.com/2/tweets` | false |
| instagram | `instagram.create_media` | POST | `https://graph.instagram.com/v25.0/{ig_user_id}/media` | false |
| instagram | `instagram.publish_media` | POST | `https://graph.instagram.com/v25.0/{ig_user_id}/media_publish` | false |

Triggers (channel/webhook cues):

| Provider | trigger id | mechanism | cue_kind | extra |
|---|---|---|---|---|
| discord | message | channel | `discord.message` | node `channel_trigger` |
| slack | message | channel | `slack.message` | node `channel_trigger` |
| telegram | message | channel | `telegram.message` | node `channel_trigger` |
| github | push | webhook | `github.event` | `webhook_scheme = github-hmac-sha256` |

(notion / x / instagram contributed no triggers — action-only.)

## If the connector plane is ever revived (deliberate non-goal today)

Connectors were removed **by choice** — the integration framework is not a
current goal. This section is a forward reference, **not** outstanding work: if
provider action-blocks/triggers are ever wanted, the plane belongs entirely to
`apxm-auth` (which already owns the provider identity/auth/category/api_base).
Recommended shape:

1. **`apxm-auth/providers.toml`** — extend each provider table with its actions
   and triggers, e.g.

   ```toml
   [discord]
   # …existing auth/category/api_base/test_capability…
   [[discord.action]]
   capability = "discord.post"
   method     = "POST"
   path       = "/channels/{channel_id}/messages"   # relative to api_base
   read_only  = false
   [[discord.trigger]]
   id        = "message"
   mechanism = "channel"
   cue_kind  = "discord.message"
   ```

   Bump `schema_version` and parse the new tables in
   `apxm-auth/crates/apxm-auth/src/providers/registry.rs`.

2. **`apxm-server`** (`crates/tools/apxm-server/src/capability.rs`) — register
   `provider.call` capabilities from the providers.toml actions (via apxm-auth)
   instead of scanning connector-pack `tools.toml` under `~/.apxm/libs`.

3. **`apxm-studio`** (`crates/apxm-studio/src/apps.rs`
   `scan_installed_apps`) — derive the app catalog (blocks + triggers) from
   providers.toml instead of scanning `[app]` packs.

4. **`apxm-os`** — read triggers from providers.toml.

Until that wiring lands, the studio connector blocks and the runtime
`provider.call` registration for these providers are dormant. This is
acceptable: the integration framework is deferred, and the source data is
preserved here and in git history (the packs existed at the commit preceding
this doc).

This does **not** affect `apxm-os-discord-curate`: that is a compiled *skill*
pack (curate/answer LLM flows), independent of the discord connector pack.
