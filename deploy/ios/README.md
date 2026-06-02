# CatGo — iOS build

Build the CatGo iOS app (iPhone / iPad). The frontend, the russh SSH/SFTP Rust
layer, and the `isMobile()` mobile UI (MobileWorkspace) are all shared with
Android — the same code runs on iOS. What's iOS-specific is the toolchain and a
couple of native patches.

> **macOS required.** iOS apps can only be built on macOS (Xcode + `codesign`).
> You cannot build or sign an iOS app from Linux/Windows. Options if you don't
> own a Mac:
> - **GitHub Actions** — `.github/workflows/ios-build.yml` builds on a hosted
>   macOS runner (simulator-unsigned by default; signed device IPA when the Apple
>   secrets are set). This is the no-Mac-purchase path.
> - **Cloud Mac** (MacStadium / MacinCloud) — rent a Mac with Xcode.

## Apple account — what you need

| Goal | Account |
|------|---------|
| Develop + run on **your own** iPhone (7-day app expiry, re-sign weekly) | Free Apple ID (still needs a Mac/Xcode) |
| **TestFlight** (give testers a build) or **App Store** | Paid Apple Developer Program — USD 99/yr |
| 1-year signing, no device limit, push, etc. | Paid program |

So: a free Apple ID is enough to *develop and side-load to your own device*; the
$99 program is for *distribution*.

## 1. Prerequisites (on the Mac)

```bash
xcode-select --install            # Xcode command-line tools (or install full Xcode)
# JS + Rust (same as desktop)
corepack enable && corepack prepare pnpm@10.28.2 --activate
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
pnpm install
```

## 2. Initialise the iOS project (one-time per checkout)

```bash
pnpm tauri ios init
```

Generates `src-tauri/gen/ios/` (the Xcode project). Like `gen/android/`, it is
**machine-local and not committed** — re-run after a fresh clone. The reverse-DNS
app id is taken from `bundle.identifier` (`com.catgo.app`).
`src-tauri/tauri.ios.conf.json` already overrides `bundle.externalBin` to `[]`
so the Python/Node desktop sidecars are not linked into the iOS build.

## 2b. Post-init native patches (REQUIRED — re-apply after every `init`)

`tauri ios init` regenerates the native project, so re-apply these after each
`init` (the CI workflow does the same so it stays reproducible).

**(1) `Info.plist`** — at `src-tauri/gen/ios/CatGo_iOS/Info.plist`:

- `ITSAppUsesNonExemptEncryption = false` — declares no custom/non-exempt crypto
  (russh uses standard SSH crypto; this avoids an App Store export-compliance
  prompt). The CI workflow sets this via PlistBuddy.
- **No camera/mic/location keys are needed** — the hand-tracking gesture mode is
  disabled on mobile, and the file picker / russh SSH do not need usage strings.
- **App Transport Security:** none required. OPTIMADE/Materials-Project fetches
  go through the Tauri HTTP plugin (native, not WKWebView), and russh SSH is a
  raw Rust TCP socket — neither is subject to WKWebView ATS. If you later add a
  plain-HTTP backend you'd add an ATS exception here.

**(2) Keyboard insets** — unlike Android, **no MainActivity-style IME patch is
needed**: WKWebView resizes `window.innerHeight` / honours `safe-area-inset-*`
for the keyboard. MobileWorkspace already has no visualViewport binding, so the
keyboard handling that works on Android should also work here. Verify on a device
and only add a tweak if a gap/overlap appears.

## 3. Run / build

```bash
# Run in the simulator or on a tethered device (dev, live reload)
pnpm tauri ios dev

# Build (.ipa). Signing needs an Apple Team; pass it in the env or set the
# development team in Xcode (open src-tauri/gen/ios/*.xcodeproj once).
pnpm tauri ios build
```

The IPA lands under `src-tauri/gen/ios/build/`. Distribute via TestFlight
(`xcrun altool` / Transporter) or the App Store with the paid program.

## Secure storage (Keychain) — follow-up

The mobile SSH-key private key and the saved cluster password are encrypted at
rest by a software AES-256-GCM envelope in `src-tauri/src/ssh/keygen.rs`
(`ssh_key_store` / `ssh_key_load`), the same cross-platform path used on Android.
On iOS this is protected by the app sandbox + (on a passcode-locked device) data
protection.

To harden to the **iOS Keychain / Secure Enclave** (so the wrapping key is
hardware-backed and non-exportable), add a `#[cfg(target_os = "ios")]` branch in
the `wrap_dek`/`unwrap_dek` seam of keygen.rs that stores the DEK via the
Security framework (the `keyring` crate's iOS backend, or `security-framework`
directly). The `KeyEnvelope` is versioned (`v`) so the format can migrate. This
mirrors the Android-Keystore follow-up noted in `deploy/android/README.md`.

## Notes

- `pnpm tauri ...` routes through `scripts/tauri-dev.mjs` (transparent
  pass-through for `ios` subcommands, same as `android`).
- The mobile UI is identical to Android (entry chooser, structure editor +
  trajectory playback, russh terminal/SFTP, database import, language switch).
  WKWebView is generally newer than the Android System WebView, so features that
  the old Android WebView lacked (e.g. WebGPU large-system mode on iOS 16.4+)
  may work on recent iPhones.
