# Website Design Refresh: Neobrutalist → Clean Modern

## Context

The Codiv website was built with a neobrutalist design system (thick 3px borders, hard drop shadows, 0px border-radius, extra-bold typography). While distinctive, this style makes the docs feel cluttered and hard to read. The user wants to soften the entire site to a clean, modern developer-docs aesthetic (like Stripe/Tailwind/Astro docs) so the transition between landing page and docs is seamless.

## Decision

**Approach A: Soften Everything** — replace neobrutalist tokens site-wide with a clean modern design system. Same layout structure, same colors, same fonts — just lighter visual weight everywhere.

## Design Tokens — Before & After

| Token | Current (Neobrutalist) | New (Clean) |
|-------|----------------------|-------------|
| `--border-width` | `3px` | `1px` |
| `--border-radius` | `0px` | `8px` |
| `--color-border` (light) | `#1a1a1a` | `#e5e7eb` |
| `--color-border` (dark) | `#f0f0f0` | `#333333` |
| `--shadow` | `4px 4px 0 var(--color-shadow)` | `0 1px 3px 0 rgba(0,0,0,0.1), 0 1px 2px -1px rgba(0,0,0,0.1)` |
| `--shadow-sm` | `2px 2px 0 var(--color-shadow)` | `0 1px 2px 0 rgba(0,0,0,0.05)` |
| `--shadow-lg` | `6px 6px 0 var(--color-shadow)` | `0 10px 15px -3px rgba(0,0,0,0.1), 0 4px 6px -4px rgba(0,0,0,0.1)` |
| heading `font-weight` | `800` | `700` |

Dark mode shadows: use `rgba(0,0,0,0.3)` / `rgba(0,0,0,0.4)` / `rgba(0,0,0,0.5)` respectively.

Dark mode border: use `#333333` (not `#2a2a2a`) for sufficient contrast against `#0a0a0a` background.

### What stays the same

- **Space Grotesk** for headings, **Inter** for body, **JetBrains Mono** for code
- **Blue accent** `#2563eb` (light) / `#60a5fa` (dark)
- **Color palette** for bg, bg-alt, text, text-muted, success, warning, danger
- **Spacing scale** (4px base)
- **Font size scale**
- **Layout structure** — all pages, grids, three-column docs layout unchanged
- **Terminal demo** — always dark, keeps its own visual style (only its outer border/shadow changes)

## File-by-File Changes

### 1. `src/styles/global.css`

**Design tokens (`:root`):**
- `--border-width: 3px` → `1px`
- `--border-radius: 0px` → `8px`
- `--color-border: #1a1a1a` → `#e5e7eb`
- `--color-shadow: #1a1a1a` → remove (shadows use rgba now)
- `--shadow` / `--shadow-sm` / `--shadow-lg` → soft blur values from table above

**Dark theme `[data-theme="dark"]`:**
- `--color-border: #f0f0f0` → `#333333`
- `--color-shadow` → remove
- `--shadow` / `--shadow-sm` / `--shadow-lg` → dark-appropriate rgba values

**Base element styles:**
- `h1-h6`: `font-weight: 800` → `700`
- `a` (links): `text-decoration-thickness: 2px` → `1px` (softer underlines)
- `code` (inline): keep `1px` border, hardcode `border-radius: 4px` (override `var(--border-radius)` which is 8px — too large for inline elements)
- `pre` (code blocks): border stays `var(--border)`, add `border-radius: var(--border-radius)`, shadow → `var(--shadow)` (now soft)
- `blockquote`: stays the same (border-left accent is clean already)
- `hr`: change from `border-top: var(--border)` to `border-top: 1px solid var(--color-bg-alt)` with a fallback heavier color — or use `2px solid var(--color-border)` to stay visible
- `table th, td`: borders auto-update via `var(--border)` — acceptable for clean look

**Component base classes:**
- `.card`: uses `var(--border)`, `var(--shadow)`, `var(--border-radius)` — all update automatically via tokens. **Hover effect**: change from `translate(-2px, -2px)` + shadow-lg to `translateY(-2px)` + shadow-lg (upward lift, not diagonal).
- `.btn`: uses vars — updates automatically. **Hover effect**: change from `translate(2px, 2px)` + shadow-sm (press down) to `translateY(-1px)` + shadow (subtle lift). **Active**: `translateY(0)` + shadow-sm.
- `.btn-primary`: `border-color: var(--color-border)` → `border-color: transparent` (primary button shouldn't have a visible gray border).
- `.badge`: `border: 2px solid` → `border: 1px solid`, add `border-radius: 9999px` (pill shape).

### 1b. `src/components/BaseHead.astro`

- Google Fonts URL: `Space+Grotesk:wght@700;800` → `Space+Grotesk:wght@700` (drop 800 weight, no longer used anywhere). Keep `Inter:wght@400;500;600` and `JetBrains+Mono:wght@400;500` unchanged.

### 2. `src/components/Header.astro`

- `.logo-text`: `font-weight: 800` → `700` (hardcoded in component, not affected by global rule).
- Header `border-bottom: var(--border)` — updates automatically (now 1px light gray).
- Mobile hamburger `.nav-toggle-label`: remove `border: var(--border)`, `box-shadow: var(--shadow-sm)`. Replace with simpler styling: no border, subtle hover background.
- Mobile nav dropdown: `border-bottom: var(--border)`, `box-shadow: var(--shadow)` → updates automatically. Remove inner `border-bottom: 1px solid var(--color-border)` on nav links (use lighter divider or none).
- SVG icons: `stroke-width="3"` → `"2"`, `stroke-linecap="square"` → `"round"` on hamburger. Same for GitHub external link arrow (`stroke-width="2.5"` → `"2"`, `stroke-linecap="square"` → `"round"`).

### 3. `src/components/ThemeToggle.astro`

- `.theme-toggle`: remove `border: var(--border)`, `box-shadow: var(--shadow-sm)`. Add `border: none`, `border-radius: 8px`. Hover: remove translate/shadow effects, use `background: var(--color-bg-alt)` only. Active: remove translate/shadow.
- Remove `:hover` rule referencing `var(--color-shadow)` (this variable is being removed).
- Monitor icon SVG: `rx="0"` → `rx="2"` (match the softer feel).
- SVG icons: `stroke-width="2.5"` → `"2"`, `stroke-linecap="square"` → `"round"` on all three icons.

### 4. `src/components/Footer.astro`

- `border-top: var(--border)` — updates automatically. No other changes needed.

### 5. `src/components/Badge.astro`

- `.badge-component`: `border: var(--border)` → `border: 1px solid transparent`, `border-radius: 0` → `9999px` (pill), remove `box-shadow: var(--shadow-sm)`.
- Color adjustments: badges get subtle background + matching text instead of harsh borders. `badge-complete`: keep green bg. `badge-planned`: lighter, more subtle.

### 6. `src/components/Hero.astro`

- `.hero-heading`: `font-weight: 800` → `700` (hardcoded in component, overrides global h1 rule).
- Buttons update via `.btn` global changes. No other structural changes.

### 7. `src/components/FeatureCard.astro`

- `.feature-title`: `font-weight: 800` → `700`. Uses `.card` class — visual updates come from global.

### 8. `src/components/FeatureGrid.astro`

- No changes needed. Layout-only component.

### 9. `src/components/RoadmapTimeline.astro`

- `.timeline-dot`: `border: var(--border)` — updates automatically (1px). This is fine.
- `.timeline-line`: `width: var(--border-width)` → this will become 1px which is too thin. Hardcode to `width: 2px`.

### 10. `src/components/TerminalDemo.astro`

- `.terminal`: `border: 3px solid var(--color-border)` → `border: 1px solid var(--color-border)`, `box-shadow: 6px 6px 0 var(--color-shadow)` → `var(--shadow-lg)`, add `border-radius: var(--border-radius)`.
- `.terminal-titlebar`: `border-bottom: 2px solid #333` → `1px solid #333`, add `border-radius: var(--border-radius) var(--border-radius) 0 0` to match container.

### 11. `src/styles/docs.css`

- `.docs-sidebar`: `border-right: var(--border)` — updates automatically (1px, light gray).
- `.sidebar-link`: `border-left: 3px solid` → `2px solid`. Active state gets `border-radius: 0 4px 4px 0`.
- `.docs-content h1`: `border-bottom: var(--border)` — updates automatically.
- `.docs-content h2`: `border-bottom: 2px solid var(--color-bg-alt)` → `1px solid var(--color-bg-alt)`.
- `.docs-content img`: `border: var(--border)`, `box-shadow: var(--shadow)` → remove shadow, keep 1px border, add `border-radius: var(--border-radius)`.
- `.docs-toc`: `border-left: var(--border)` — updates automatically.
- `.docs-nav-link`: `border: var(--border)`, `box-shadow: var(--shadow-sm)` — update automatically. Add `border-radius: var(--border-radius)`. Hover: change from `translate(-2px, -2px)` + shadow to `translateY(-2px)` + shadow.
- Mobile sidebar toggle: remove aggressive border styling, use subtle background toggle.
- `DocsLayout.astro` sidebar toggle SVG: `stroke-width="3"` → `"2"`, `stroke-linecap="square"` → `"round"`.

### 12. `src/components/DocsSidebar.astro`

- No changes needed — styling comes from docs.css.

### 13. `src/components/TableOfContents.astro`

- No changes needed — styling comes from docs.css.

### 14. `src/layouts/BaseLayout.astro` & `src/layouts/DocsLayout.astro`

- No changes needed — layout-only files.

### 15. `src/pages/index.astro`, `docs/index.astro`, `docs/[...slug].astro`

- No changes needed — page assembly files.

### 16. Comment updates

- `global.css` line 2: `Neobrutalist Design System` → `Clean Modern Design System`
- `global.css` line 279: `Component Base Classes — Neobrutalist` → `Component Base Classes`

## Summary of Scope

- **Primary changes**: `global.css` (design tokens + component classes), `docs.css` (docs-specific overrides)
- **Component touch-ups**: Header, ThemeToggle, Badge, FeatureCard, Hero, TerminalDemo, RoadmapTimeline, BaseHead — removing hardcoded brutalist values, softening SVG strokes
- **No structural changes**: All layouts, pages, and content remain identical
- **No content changes**: Zero doc files affected

## Verification

1. `cd website && npm run build` — zero errors
2. `npm run dev` — visually check:
   - Landing page: hero, feature cards, roadmap, terminal demo
   - Docs: sidebar, content, code blocks, TOC
   - Light mode and dark mode
   - Mobile responsive (375px, 768px, 1200px+)
3. Confirm terminal demo still looks good (always dark, rounded corners now)
4. Confirm badges render as pills with correct colors
