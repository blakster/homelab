# homelab — interface system

Terminal-first. Every rule below is enforced in `src/ui/` and checked by `src/ui/tests.rs`.

## Intent
One glance answers "is anything broken, and what's the root cause?". Everything else is secondary.

## Focal point
- A fault banner (`▲ …`) owns row 2 when anything is Down or Blocked. It names the *root* fault, not every symptom.
- Without faults, the hub card (this Mac) is the anchor.

## Tokens (domain-named)
| token   | dark      | light     | use |
|---------|-----------|-----------|-----|
| ground  | `#0b0b0d` | `#fafafa` | canvas |
| ink     | `#e8e8ec` | `#18181b` | names, values that matter |
| ink2    | `#b6b6bf` | `#3f3f46` | secondary values |
| ink3    | `#85858f` | `#6b6b74` | metadata, labels |
| mute    | `#5a5a64` | `#9f9fa8` | disabled, hints |
| wire    | `#2a2a31` | `#e2e2e6` | borders, section rules |
| raised  | `#16161b` | `#f0f0f2` | selection |
| signal  | `#6672e8` | `#4453c9` | the one accent (charts, hub) |
| live    | `#3fb27f` | `#1f9d63` | up / online |
| relay   | `#d49a3a` | `#b7791f` | DERP-relayed paths |
| fault   | `#e5484d` | `#d92d3a` | down / blocked only |
| asleep  | `#3c3c44` | `#c8c8ce` | offline peers |

60/30/10: ground / ink tiers / signal. Status colours are semantic, never decorative.
`--plain` (or `p`) swaps truecolor for the ANSI-16 palette and text bars.

## Type and spacing
- Four text tiers via colour and weight (ink bold → ink → ink3 → mute), never size.
- 1-cell gutters; sections separated by a titled rule, depth by borders only.
- Numbers are fixed width (`latency` is always 7 cells) so columns don't jitter.
- Anything that can be long is truncated with `…` at its column width (CJK/emoji aware).

## States
- Loading: say what we're waiting for (`waiting for tailscale status…`).
- Empty: one sentence plus one next action (`No services yet. Add a [[service]] …`).
- Error: inline where the data would be, and in the banner if it's a fault.
- Off: disabled services are muted with "disabled in config" and are never faults.

## Motion
None on data ticks. Redraw only on snapshot change, input, or a 30 s clock. Bloom off.

## Widths
Layouts are designed and snapshot-tested at 50, 80, 120 and 200 columns.
- ≥120: rail + Machines topology + Services + History.
- 80–119: stacked hub, Tailnet, Services, History if room.
- <80: stacked hub, Tailnet, Services.
- Below the minimum size: a "homelab needs W×H" notice instead of a broken layout.

## Dither Kit
Charts, meters, sparklines and peer avatars come from `dither-ratatui` (half-block truecolor).
The chart legend re-inks labels from our palette because upstream ignores the theme there.
