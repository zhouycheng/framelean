# Legal Materials

This directory is the canonical home for FrameLean's release legal materials.
The repository root keeps LICENSE as the standard project license discovery
entry point. The short distribution notice lives in legal/NOTICE.md, and
release packages include both files together with this directory.

## Package Layout

Binary release packages carry these materials:

- LICENSE
- legal/NOTICE.md
- legal/COPYING
- legal/THIRD_PARTY_NOTICES.md
- legal/SOURCE_OFFER.md
- legal/third-party/

macOS release builds copy this directory into:

```text
FrameLean.app/Contents/Resources/legal/
```

The macOS ZIP contains the `.app` bundle, including these legal materials.

The Windows packaging script is configured to place `legal/` beside the Runner executable in the ZIP:

```text
<Runner executable>
legal/
```

## Maintenance

Update this directory when changing bundled runtimes, direct dependencies,
license policy, or release packaging. Keep the root LICENSE file as the full
project license text and legal/NOTICE.md as the short distribution notice.
