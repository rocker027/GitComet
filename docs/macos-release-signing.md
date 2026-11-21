# macOS Release Signing

This repository already contains the basic macOS release flow:

- [`scripts/package-macos.sh`](../scripts/package-macos.sh) builds the macOS app bundle, signs the bundled executable and app when a signing identity is provided, and emits the macOS `.dmg` artifact.
- [`scripts/notarize-macos.sh`](../scripts/notarize-macos.sh) submits the packaged DMG to Apple's notary service, staples the resulting ticket to the staged `.app` bundle and DMG, and validates the stapled artifacts.

## What You Need

For outside-the-App-Store distribution you need:

1. An Apple Developer Program team.
2. A `Developer ID Application` certificate with the private key available to `codesign`.
3. A notarization credential for `notarytool`.
   Recommended in this repo: an App Store Connect API key.
   Local alternative: a stored `notarytool` keychain profile.

Check whether the current shell can see a usable signing identity:

```bash
security find-identity -v -p codesigning
```

For release signing, this must show a valid `Developer ID Application: ...` identity. If it shows `0 valid identities found`, fix the certificate/private-key installation in Keychain Access before trying to sign.

## Local Release Flow

First, verify that macOS can see the correct signing identity:

```bash
security find-identity -v -p codesigning
```

You should see a valid `Developer ID Application: ...` entry.

Build and sign on each native architecture you ship:

```bash
scripts/package-macos.sh \
  --version 0.2.0 \
  --arch arm64 \
  --release \
  --codesign-identity "Developer ID Application: Example, Inc. (TEAMID)"
```

Then notarize and staple the generated artifacts:

```bash
scripts/notarize-macos.sh \
  --version 0.2.0 \
  --arch arm64 \
  --keychain-profile revweave-notary
```

If you prefer direct API-key authentication instead of a stored profile:

```bash
scripts/notarize-macos.sh \
  --version 0.2.0 \
  --arch arm64 \
  --api-key /path/to/AuthKey_ABC1234567.p8 \
  --key-id ABC1234567 \
  --issuer 00000000-0000-0000-0000-000000000000
```

To create a local keychain profile up front:

```bash
xcrun notarytool store-credentials revweave-notary \
  --key /path/to/AuthKey_ABC1234567.p8 \
  --key-id ABC1234567 \
  --issuer 00000000-0000-0000-0000-000000000000
```

Repeat the same packaging and notarization flow on Intel macOS for `--arch x86_64`.

## Local Verification Checklist

The packaging and notarization scripts already run the important checks for you:

- [`scripts/package-macos.sh`](../scripts/package-macos.sh) signs with Hardened Runtime and runs `codesign --verify` on the app and standalone binary.
- [`scripts/notarize-macos.sh`](../scripts/notarize-macos.sh) submits the DMG with `notarytool`, staples the `.app` and `.dmg`, validates stapling, then runs `codesign --verify` and `spctl --assess`.

After a successful local run, you can manually spot-check the finished artifacts:

```bash
codesign --verify --deep --strict --verbose=2 dist/stage/revweave-v0.2.0-macos-arm64/RevWeave.app
spctl --assess --type open --context context:primary-signature --verbose=4 dist/stage/revweave-v0.2.0-macos-arm64/RevWeave.app
spctl --assess --type open --context context:primary-signature --verbose=4 dist/revweave-v0.2.0-macos-arm64.dmg
xcrun stapler validate dist/stage/revweave-v0.2.0-macos-arm64/RevWeave.app
xcrun stapler validate dist/revweave-v0.2.0-macos-arm64.dmg
```

If all of those commands return success, the signed macOS app bundle and DMG are in good shape.

## Signing Credentials

RevWeave has no hosted CI, so signing and notarization run on a local macOS machine. Keep the signing certificate and the notary key in your login keychain or a secrets manager, never in the repository.

1. Export the `Developer ID Application` identity from Keychain Access as a `.p12` only if you need to move it to another machine. Choose an export password for that `.p12`.
2. Pass the exact identity string, for example `Developer ID Application: Example, Inc. (TEAMID)`, to `--codesign-identity`.
3. Provide notarization credentials through a stored `notarytool` keychain profile (`--keychain-profile`) or the App Store Connect API key (`--api-key`, `--key-id`, `--issuer`).

## App Store Connect Notary Key

Notarization works with an App Store Connect API key.

Create a Team API key, download the `.p8` file once, and store the key material securely.

For local use you can either:

- use the `.p8` file directly with [`scripts/notarize-macos.sh`](../scripts/notarize-macos.sh), or
- save the credentials into your macOS keychain with `xcrun notarytool store-credentials`.

## Artifact Expectations

Treat the DMG as the canonical trusted macOS download.

- The DMG is the notarized and stapled end-user artifact.
- The DMG is the macOS installation artifact. The macOS binary is not published as a separate `.tar.gz`: it would duplicate the binary the DMG already ships, and a bare copy is the one file that cannot be notarized, since Apple's notary service does not accept `.tar.gz` uploads. Users who want `revweave` on `PATH` can link `RevWeave.app/Contents/MacOS/revweave` themselves.

If you want a separately trusted CLI-only macOS artifact, publish it in a notary-supported container such as a ZIP, DMG, or signed flat PKG and update downstream packaging accordingly.
