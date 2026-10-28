# macOS IME language status research

Status: Archived

This research determines how the Rust TUI can display the active macOS input
language while running in Apple Terminal. It records disposable-probe evidence
and the technical direction later confirmed by the owner; it does not
implement the production feature.

## Goal

Determine a supported, permission-conscious Rust integration that reports the
currently selected macOS input language and keeps a compact TUI indicator
current when the user changes input sources.

The intended user-visible result is language-level status such as `IME: EN` or
`IME: JA` in the TUI. Phase 1 does not need to distinguish Japanese Hiragana,
Katakana, or alphanumeric conversion modes.

## Result and recommendation

Use Text Input Source Services as the primary integration. A small macOS FFI
boundary reads `TISCopyCurrentKeyboardInputSource`, gets the first
`kTISPropertyInputSourceLanguages` value, and listens for
`kTISNotifySelectedKeyboardInputSourceChanged`. It normalizes `en` to `IME: EN`
and `ja` to `IME: JA`; other values may use their uppercased language code.
This label intentionally says nothing about Japanese conversion sub-modes.

If notification registration fails, poll the same supported query every 500
ms while the TUI is active. If the property is absent or the query fails, show
`IME: ?` and keep editing available. Do not read HIToolbox preference files as
a fallback.

The preferred Rust approach is minimal direct FFI to Carbon/HIToolbox and Core
Foundation. The boundary is limited to property reads, notification
registration/removal, CF ownership, and string/array conversion. The macOS 26
SDK marks these symbols available since macOS 10.5, so the feature adds no
constraint beyond Phase 1's macOS 26 baseline.

## Probe results

The disposable Rust probe ran on macOS 26.7.1 (25G241), Apple Silicon, with
Rust 1.98.0 and Xcode 27.1. It linked Carbon/HIToolbox and Core Foundation and
ran in Apple Terminal.

- It reported the active ABC source as `language=en`.
- Japanese was installed but not enabled. Read-only enumeration returned
  `language=ja` for Japanese Kana, Romaji, Hiragana, and Katakana sources. No
  keyboard setting was enabled or disabled for the test.
- A reversible test used the already-enabled Korean source and restored ABC.
  The probe observed `en → ko → en` without restart. Notification and 100 ms
  polling detection both occurred in the same sampling interval.
- A 30.82-second idle run at the deliberately aggressive 100 ms polling rate
  used 0.03 seconds user CPU and 0.07 seconds system CPU. The 500 ms fallback
  is more conservative; notification delivery remains the default.
- No Accessibility, Input Monitoring, Automation, or other privacy permission
  was requested by the query, notification, or polling probe.
- A terminal-hosted AppKit probe returned `NSTextInputContext.current == nil`.
  AppKit's notification exists, but its missing context makes its selected
  source unsuitable for this non-AppKit TUI.

The lack of a live Japanese switch is a named limitation: Japanese was not an
enabled user source. The same supported property returned `ja` for installed
Japanese sources, and live process-global delivery was verified in both
directions with an enabled non-English source. Recheck a live `en ↔ ja` switch
during implementation acceptance on a machine where Japanese is enabled; it
does not block the integration choice.

## Scope

### In scope

- macOS on Apple Silicon using Apple Terminal.
- Reading the active keyboard input source and deriving its language.
- Detecting English/Japanese input-source changes while the TUI is running.
- A disposable Rust probe that does not become production code.
- Framework linkage, threading or run-loop requirements, macOS permissions,
  dependency licensing, and minimum-version implications.
- A recommendation and fallback suitable for later architecture planning.

### Out of scope

- Production code that switches the user's input source. The disposable
  verification helper temporarily selected an already-enabled source and
  restored the original after one second; it is not a product mechanism.
- Displaying conversion state, marked text, candidate windows, or Japanese
  sub-modes.
- iTerm2, Kitty, WezTerm, Ghostty, or other terminal emulators.
- Linux, Windows, or iOS input-source behavior.
- Production modules, stable interfaces, or dependency approval.

## APIs and Rust approaches to evaluate

### Text Input Source Services

Evaluate the process-global macOS Text Input Source Services API, including:

- `TISCopyCurrentKeyboardInputSource` for the current source;
- source ID, language, and localized-name properties;
- the selected-input-source change notification;
- the Carbon/HIToolbox and Core Foundation frameworks required by Rust FFI.

This is the leading candidate because the TUI is hosted by Apple Terminal and
is not itself an AppKit text-input client. The research must verify that it uses
supported symbols on the minimum macOS version rather than treating observed
behavior as a guarantee.

### AppKit text input context

Evaluate `NSTextInputContext` through `objc2-app-kit`, including its selected
keyboard input source and keyboard-selection-change notification. Apple exposes
both capabilities, but the current context may depend on an active AppKit text
client. Confirm whether a terminal process receives a useful context before
recommending this route.

Do not read `com.apple.HIToolbox` preference files or parse `defaults` output as
the primary implementation. Such inspection may be used only to diagnose a
probe result and must not become the recommendation.

## Probe method

Create the probe in a disposable directory outside the repository so P1-M1
does not add production dependencies.

1. Print a normalized language code, input-source identifier, and localized
   name for the initially selected input source. The probe must not print typed
   text or observe keystrokes.
2. While the probe runs in Apple Terminal, switch between the enabled English
   and Japanese input sources using normal macOS controls.
3. Record whether a supported notification updates the value without a
   keyboard event reaching the TUI.
4. If notification delivery is not viable for a terminal-hosted Rust process,
   measure a low-frequency polling fallback and its effect on idle CPU use.
5. Repeat changes in both directions and while Apple Terminal remains focused.
6. Record whether macOS requests Accessibility, Input Monitoring, Automation,
   or another privacy permission.
7. Compare minimal system-framework FFI with `objc2-app-kit` for API coverage,
   unsafe-code surface, main-thread/run-loop needs, licenses, and MSRV.

Baseline environment for the first probe:

| Item | Baseline |
|---|---|
| macOS | 26.7.1 (25G241) |
| Architecture | Apple Silicon (`arm64`) |
| Terminal | Apple Terminal |
| Rust | 1.98.0 |

Results must name the exact tested environment and must not generalize one
machine's observation to every supported macOS version.

## Evidence to record

- Initial English and Japanese language detection.
- Live change behavior and update latency.
- Whether language data comes from an API property or a documented mapping from
  a stable source identifier.
- Notification delivery requirements, or polling interval and measured idle
  cost when notification delivery is unavailable.
- Required macOS frameworks and permissions.
- Rust dependency versions, feature flags, MSRV, licenses, and relevant unsafe
  boundary.
- Behavior when an input source supplies no language property, without turning
  this research into a complete exception-handling design.

Screenshots and logs must not contain diary text. Input-source identifiers and
language codes are acceptable research evidence; typed content is not.

## Completion checklist

- [x] Official Apple documentation for the evaluated APIs and notifications is
  cited.
- [x] A disposable Rust probe reports active English and installed Japanese
  through the supported language property in Apple Terminal.
- [x] The probe observes enabled-language changes in both directions without
  restarting; the unavailable live Japanese switch is documented above.
- [x] The research records update latency, idle cost, framework requirements,
  and permission behavior.
- [x] Text Input Source Services and `NSTextInputContext` are compared with
  evidence from the terminal-hosted probe.
- [x] One preferred integration and one bounded fallback are recommended for
  later architecture planning.
- [x] The proposed TUI label is language-only and does not claim to report
  Japanese conversion sub-modes.
- [x] No production dependency or later milestone architecture is added.

## Sources

- [Apple: `NSTextInputContext`](https://developer.apple.com/documentation/appkit/nstextinputcontext)
- [Apple: keyboard selection change notification](https://developer.apple.com/documentation/AppKit/NSTextInputContext/keyboardSelectionDidChangeNotification)
- [Apple: text input source identifiers](https://developer.apple.com/documentation/appkit/nstextinputcontext/keyboardinputsources)
- [Apple: third-party input method management](https://developer.apple.com/library/archive/qa/qa1810/_index.html)
- [`objc2-app-kit`: `NSTextInputContext`](https://docs.rs/objc2-app-kit/latest/objc2_app_kit/struct.NSTextInputContext.html)
- macOS 26 SDK `HIToolbox/TextInputSources.h`, inspected locally on
  2026-10-28 for availability annotations and notification/property symbols.
- [P1-M1 validation evidence](p1-m1-validation-evidence.md)
