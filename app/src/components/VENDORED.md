# Vendored components

Copied as source (shadcn-registry style), not installed as packages, so they can be read, themed and changed. Fetched 2026-11-23 and scanned before use: no network, storage, clipboard or `eval` calls; imports are only `react`, `react-dom`, `motion` and `@hugeicons/*`.

| Folder | Source | Licence |
| --- | --- | --- |
| `rb/` | [React Bits](https://reactbits.dev), `<Name>-TS-TW` registry items | MIT + Commons Clause ([`rb/LICENSE.md`](rb/LICENSE.md)). Fine to ship inside Tether; not fine to resell the components on their own. |
| `magic/` | [Magic UI](https://magicui.design) registry | MIT ([`magic/LICENSE.md`](magic/LICENSE.md)) |

| Component | Used for |
| --- | --- |
| `rb/PillNav` | Top navigation (glass bar, gliding highlight) |
| `rb/SpotlightCard` | `Card` surfaces that catch a light following the cursor |
| `rb/SpecularButton` | `PrimaryAction`, the one hero button per page (WebGL edge highlight) |
| `rb/WarmTooltip` | Icon-button tooltips; after the first, neighbours open instantly |
| `rb/RubberSegment` | `Segmented` filters |
| `rb/SquishSwitch` | `Switch` on the Modules page |
| `rb/CodeSlots` | Entering a pairing code |
| `rb/SlideCommit` | Slide-to-revoke a device |
| `rb/SwipeToast` | Notifications, bottom-right like Windows |
| `rb/BlurText` | Overview headline reveal |
| `magic/animated-beam` | Device constellation on the Overview |
| `magic/border-beam` | Light running round an active transfer |
| `magic/number-ticker` | `Figure`, large springing numbers |

Theme them through the wrappers in `../ui.tsx`, not by editing these files, so they can be refreshed from upstream. Libraries that need hex colours (shaders, contrast maths) get them from `palette` in `ui.tsx`, which mirrors `index.css`.
