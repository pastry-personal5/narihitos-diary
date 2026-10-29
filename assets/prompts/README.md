# AI prompt assets

Status: Active

Every prompt sent to an AI provider lives here, never in code. Each file pairs
a provider, a model, a reasoning effort, and the prompt text. Layout:
`assets/prompts/<provider>/<action>.md`.

File format: a `---` front matter block with `provider`, `model`, and
`reasoning_effort`, then the prompt body (one trailing newline is dropped).

- `model` / `reasoning_effort`: `default` means the provider/profile default
  applies and the field is omitted from the request. Any other value is sent
  (Codex `turn/start` `model` / `effort`). Apple Foundation Models has one
  on-device model and no effort setting, so its effort is `not-applicable`.
- Placeholders: `{{locale}}` (BCP-47 writing locale) and `{{data}}` (the JSON
  request payload). Substitution is single-pass, so diary text containing
  `{{...}}` is never expanded.
- Rust embeds the files with `include_str!` (`diary-core::prompt_asset`). The
  Swift helper receives the raw templates in the bridge request and renders
  them itself, so the prompt text exists only here.
